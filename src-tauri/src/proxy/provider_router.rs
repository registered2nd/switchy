//! Provider router
//!
//! Selects and manages proxy target providers, with failover

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider router
pub struct ProviderRouter {
    /// Database connection
    db: Arc<Database>,
    /// Circuit breakers, keyed "app_type:provider_id"
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
}

impl ProviderRouter {
    /// Create a provider router
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The database this router reads providers from.
    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    /// Select available providers (with failover)
    ///
    /// Returns available providers in priority order:
    /// - failover off: only the current provider
    /// - failover on: only the failover queue, current provider first, then the rest in queue order (P1 → P2 → ...)
    ///
    /// `model` is the model the request asks for; rotation counts only the
    /// quota windows that apply to it.
    pub async fn select_providers(
        &self,
        app_type: &str,
        model: Option<&str>,
    ) -> Result<Vec<Provider>, AppError> {
        let mut result = Vec::new();
        let mut total_providers = 0usize;
        let mut circuit_open_count = 0usize;

        if app_type == "codex" {
            super::codex_pool::file_live_login(&self.db);
        }

        // Check whether auto failover is on for this app (read from the proxy_config table)
        let auto_failover_enabled = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(config) => config.auto_failover_enabled,
            Err(e) => {
                log::error!(
                    "[{app_type}] Failed to read proxy_config: {e}; failover disabled by default"
                );
                false
            }
        };

        if auto_failover_enabled {
            // Failover on: try providers in queue order (P1 → P2 → ...)
            let all_providers = self.db.get_all_providers(app_type)?;

            // Use the DAO's ordering so it matches what the frontend shows
            let ordered_ids: Vec<String> = self
                .db
                .get_failover_queue(app_type)?
                .into_iter()
                .map(|item| item.provider_id)
                .collect();

            total_providers = ordered_ids.len();

            for provider_id in ordered_ids {
                let Some(provider) = all_providers.get(&provider_id).cloned() else {
                    continue;
                };

                let circuit_key = format!("{app_type}:{}", provider.id);
                let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;

                if breaker.is_available().await {
                    result.push(provider);
                } else {
                    circuit_open_count += 1;
                }
            }
        } else {
            // Failover off: use only the current provider, skip the circuit breaker check
            let current_id = AppType::from_str(app_type)
                .ok()
                .and_then(|app_enum| {
                    crate::settings::get_effective_current_provider(&self.db, &app_enum)
                        .ok()
                        .flatten()
                })
                .or_else(|| self.db.get_current_provider(app_type).ok().flatten());

            if let Some(current_id) = current_id {
                if let Some(current) = self.db.get_provider_by_id(&current_id, app_type)? {
                    total_providers = 1;
                    result.push(current);
                }
            }
        }

        // Quota-driven rotation: accounts at their limit go to the back of the queue.
        if matches!(app_type, "codex" | "claude") && result.len() > 1 {
            let pool = self.db.get_account_pool_config().unwrap_or_default();
            if pool.enabled {
                result = super::account_pool::order_by_quota(result, model, pool.threshold_percent);
            }
        }

        if result.is_empty() {
            if total_providers > 0 && circuit_open_count == total_providers {
                log::warn!("[{app_type}] [FO-004] All providers are circuit-broken");
                return Err(AppError::AllProvidersCircuitOpen);
            } else {
                log::warn!("[{app_type}] [FO-005] No provider configured");
                return Err(AppError::NoProvidersConfigured);
            }
        }

        Ok(result)
    }

    /// Acquire a circuit breaker permit before running a request
    ///
    /// - Closed: allow
    /// - Open: once the timeout passes, switch to HalfOpen and allow one probe
    /// - HalfOpen: allow probes per the rate limit
    ///
    /// The caller must release the HalfOpen slot with `record_result()` after the request,
    /// or the provider can stay unable to enter the probe state for a long time.
    pub async fn allow_provider_request(&self, provider_id: &str, app_type: &str) -> AllowResult {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.allow_request().await
    }

    /// Record a provider request result
    pub async fn record_result(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), AppError> {
        // 1. Get this app's circuit breaker config
        let failure_threshold = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => app_config.circuit_failure_threshold,
            Err(_) => 5, // default
        };

        // 2. Update circuit breaker state
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;

        if success {
            breaker.record_success(used_half_open_permit).await;
        } else {
            breaker.record_failure(used_half_open_permit).await;
        }

        // 3. Update health status in the database (using the configured threshold)
        self.db
            .update_provider_health_with_threshold(
                provider_id,
                app_type,
                success,
                error_msg.clone(),
                failure_threshold,
            )
            .await?;

        Ok(())
    }

    /// Reset circuit breakers (manual recovery)
    pub async fn reset_circuit_breaker(&self, circuit_key: &str) {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(circuit_key) {
            breaker.reset().await;
        }
    }

    /// Reset the circuit breaker for one provider
    pub async fn reset_provider_breaker(&self, provider_id: &str, app_type: &str) {
        let circuit_key = format!("{app_type}:{provider_id}");
        self.reset_circuit_breaker(&circuit_key).await;
    }

    /// Release the HalfOpen permit only, without touching health stats (neutral)
    ///
    /// For cases such as rectifiers: the result should not count toward provider health,
    /// but the probe slot must still be released so HalfOpen does not get stuck
    pub async fn release_permit_neutral(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
    ) {
        if !used_half_open_permit {
            return;
        }
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.release_half_open_permit();
    }

    /// Update the config of all circuit breakers (hot reload)
    pub async fn update_all_configs(&self, config: CircuitBreakerConfig) {
        let breakers = self.circuit_breakers.read().await;
        for breaker in breakers.values() {
            breaker.update_config(config.clone()).await;
        }
    }

    /// Get circuit breaker state
    #[allow(dead_code)]
    pub async fn get_circuit_breaker_stats(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Option<crate::proxy::circuit_breaker::CircuitBreakerStats> {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breakers = self.circuit_breakers.read().await;

        if let Some(breaker) = breakers.get(&circuit_key) {
            Some(breaker.get_stats().await)
        } else {
            None
        }
    }

    /// Get or create a circuit breaker
    async fn get_or_create_circuit_breaker(&self, key: &str) -> Arc<CircuitBreaker> {
        // Try the read lock first
        {
            let breakers = self.circuit_breakers.read().await;
            if let Some(breaker) = breakers.get(key) {
                return breaker.clone();
            }
        }

        // Not found: take the write lock to create it
        let mut breakers = self.circuit_breakers.write().await;

        // Double-check to avoid a race
        if let Some(breaker) = breakers.get(key) {
            return breaker.clone();
        }

        // Extract app_type from the key (format: "app_type:provider_id")
        let app_type = key.split(':').next().unwrap_or("claude");

        // Read this app's circuit breaker config
        let config = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => crate::proxy::circuit_breaker::CircuitBreakerConfig {
                failure_threshold: app_config.circuit_failure_threshold,
                success_threshold: app_config.circuit_success_threshold,
                timeout_seconds: app_config.circuit_timeout_seconds as u64,
                error_rate_threshold: app_config.circuit_error_rate_threshold,
                min_requests: app_config.circuit_min_requests,
            },
            Err(_) => crate::proxy::circuit_breaker::CircuitBreakerConfig::default(),
        };

        let breaker = Arc::new(CircuitBreaker::new(config));
        breakers.insert(key.to_string(), breaker.clone());

        breaker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("SWITCHY_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("SWITCHY_TEST_HOME", dir.path());
            crate::settings::reload_settings().expect("reload settings");

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("SWITCHY_TEST_HOME", value),
                None => env::remove_var("SWITCHY_TEST_HOME"),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_provider_router_creation() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let router = ProviderRouter::new(db);

        let breaker = router.get_or_create_circuit_breaker("claude:test").await;
        assert!(breaker.allow_request().await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_disabled_uses_current_provider() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router.select_providers("claude", None).await.unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "a");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_serves_the_current_provider_first_then_the_queue_order() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // Set sort_index to control order: b=1, a=2
        let mut provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        provider_a.sort_index = Some(2);
        let mut provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);
        provider_b.sort_index = Some(1);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();

        db.add_to_failover_queue("claude", "b").unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // Enable auto failover (via the new proxy_config API)
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router.select_providers("claude", None).await.unwrap();

        assert_eq!(providers.len(), 2);
        // The provider enabled by hand is served first; the rest follow in
        // queue order.
        assert_eq!(providers[0].id, "a");
        assert_eq!(providers[1].id, "b");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_uses_queue_only_even_if_current_not_in_queue() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let mut provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);
        provider_b.sort_index = Some(1);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();

        // Only b is in the failover queue (the common setup where the current provider is not queued)
        db.add_to_failover_queue("claude", "b").unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router.select_providers("claude", None).await.unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "b");
    }

    #[tokio::test]
    #[serial]
    async fn test_select_providers_does_not_consume_half_open_permit() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();

        db.add_to_failover_queue("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        // Enable auto failover (via the new proxy_config API)
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        router
            .record_result("b", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        let providers = router.select_providers("claude", None).await.unwrap();
        assert_eq!(providers.len(), 2);

        assert!(router.allow_provider_request("b", "claude").await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_release_permit_neutral_frees_half_open_slot() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // Circuit breaker config: trip after 1 failure, 0s timeout goes straight to HalfOpen
        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        db.save_provider("claude", &provider_a).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // Enable auto failover
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        // Trip the breaker: 1 failure
        router
            .record_result("a", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        // First request: takes the HalfOpen probe slot
        let first = router.allow_provider_request("a", "claude").await;
        assert!(first.allowed);
        assert!(first.used_half_open_permit);

        // Second request should be rejected (slot taken)
        let second = router.allow_provider_request("a", "claude").await;
        assert!(!second.allowed);

        // Release the slot with release_permit_neutral (does not affect health stats)
        router
            .release_permit_neutral("a", "claude", first.used_half_open_permit)
            .await;

        // Third request should be allowed (slot released)
        let third = router.allow_provider_request("a", "claude").await;
        assert!(third.allowed);
        assert!(third.used_half_open_permit);
    }
}
