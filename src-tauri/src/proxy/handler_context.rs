//! Request context
//!
//! Context for the request lifecycle; wraps the shared initialization

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::proxy::{
    extract_session_id,
    forwarder::RequestForwarder,
    server::ProxyState,
    types::{AppProxyConfig, CopilotOptimizerConfig, OptimizerConfig, RectifierConfig},
    ProxyError,
};
use axum::http::HeaderMap;
use std::time::Instant;

/// Streaming timeout configuration
#[derive(Debug, Clone, Copy)]
pub struct StreamingTimeoutConfig {
    /// First-byte timeout (seconds); 0 disables it
    pub first_byte_timeout: u64,
    /// Idle timeout (seconds); 0 disables it
    pub idle_timeout: u64,
}

/// Request context
///
/// Lives for the whole request and holds:
/// - timing
/// - the app's proxy configuration (per app)
/// - the selected providers (for failover)
/// - the requested model name
/// - the log tag
/// - the session ID (for log correlation)
pub struct RequestContext {
    /// Request start time
    pub start_time: Instant,
    /// The app's proxy configuration (per app; includes retry count and timeouts)
    pub app_config: AppProxyConfig,
    /// The selected provider (first in the failover chain)
    pub provider: Provider,
    /// Full provider list (for failover)
    providers: Vec<Provider>,
    /// The "current provider" when the request started (decides whether the UI/tray need syncing)
    ///
    /// This is the device-level current provider from the local settings.
    /// In proxy mode, if the provider actually used differs from it, a switch is triggered so the UI stays accurate.
    pub current_provider_id: String,
    /// Model name in the request
    pub request_model: String,
    /// Log tag (e.g. "Claude", "Codex", "Gemini")
    pub tag: &'static str,
    /// App type string (e.g. "claude", "codex", "gemini")
    pub app_type_str: &'static str,
    /// App type (reserved; currently used via app_type_str)
    #[allow(dead_code)]
    pub app_type: AppType,
    /// Session ID (taken from the client request or newly generated)
    pub session_id: String,
    /// Rectifier configuration
    pub rectifier_config: RectifierConfig,
    /// Optimizer configuration
    pub optimizer_config: OptimizerConfig,
    /// Copilot optimizer configuration
    pub copilot_optimizer_config: CopilotOptimizerConfig,
}

impl RequestContext {
    /// Creates the request context
    ///
    /// # Arguments
    /// * `state` - proxy server state
    /// * `body` - request body JSON
    /// * `headers` - request headers (used to extract the session ID)
    /// * `app_type` - app type
    /// * `tag` - log tag
    /// * `app_type_str` - app type string
    ///
    /// # Errors
    /// Returns `ProxyError` if provider selection fails
    pub async fn new(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
    ) -> Result<Self, ProxyError> {
        let start_time = Instant::now();

        // Read the app's proxy configuration (per app) from the database
        let app_config = state
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        // Read the rectifier configuration from the database
        let rectifier_config = state.db.get_rectifier_config().unwrap_or_default();
        let optimizer_config = state.db.get_optimizer_config().unwrap_or_default();
        let copilot_optimizer_config = state.db.get_copilot_optimizer_config().unwrap_or_default();

        let current_provider_id =
            crate::settings::get_current_provider(&app_type).unwrap_or_default();

        // Extract the model name from the request body
        let request_model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Extract the session ID
        let session_result = extract_session_id(headers, body, app_type_str);
        let session_id = session_result.session_id.clone();

        log::debug!(
            "[{}] Session ID: {} (from {:?}, client_provided: {})",
            tag,
            session_id,
            session_result.source,
            session_result.client_provided
        );

        // Select providers with the shared ProviderRouter (circuit breaker state persists across requests)
        // Note: called only once here and passed to the forwarder, so HalfOpen slots are not consumed twice
        let providers = state
            .provider_router
            .select_providers(
                app_type_str,
                Some(request_model.as_str()).filter(|m| *m != "unknown"),
            )
            .await
            .map_err(|e| match e {
                crate::error::AppError::AllProvidersCircuitOpen => {
                    ProxyError::AllProvidersCircuitOpen
                }
                crate::error::AppError::NoProvidersConfigured => ProxyError::NoProvidersConfigured,
                _ => ProxyError::DatabaseError(e.to_string()),
            })?;

        let provider = providers
            .first()
            .cloned()
            .ok_or(ProxyError::NoAvailableProvider)?;

        log::debug!(
            "[{}] Provider: {}, model: {}, failover chain: {} providers, session: {}",
            tag,
            provider.name,
            request_model,
            providers.len(),
            session_id
        );

        Ok(Self {
            start_time,
            app_config,
            provider,
            providers,
            current_provider_id,
            request_model,
            tag,
            app_type_str,
            app_type,
            session_id,
            rectifier_config,
            optimizer_config,
            copilot_optimizer_config,
        })
    }

    /// Extracts the model name from the URI (Gemini only)
    ///
    /// The Gemini API carries the model name in the URI, for example:
    /// `/v1beta/models/gemini-pro:generateContent`
    pub fn with_model_from_uri(mut self, uri: &axum::http::Uri) -> Self {
        let endpoint = uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(uri.path());

        self.request_model = endpoint
            .split('/')
            .find(|s| s.starts_with("models/"))
            .and_then(|s| s.strip_prefix("models/"))
            .map(|s| s.split(':').next().unwrap_or(s))
            .unwrap_or("unknown")
            .to_string();

        self
    }

    /// Creates the RequestForwarder
    ///
    /// Uses the shared ProviderRouter so circuit breaker state persists across requests
    ///
    /// Configuration rules:
    /// - Failover on: timeouts apply as configured (0 disables a timeout)
    /// - Failover off: timeouts do not apply (all passed as 0)
    pub fn create_forwarder(&self, state: &ProxyState) -> RequestForwarder {
        let (non_streaming_timeout, first_byte_timeout, idle_timeout) =
            if self.app_config.auto_failover_enabled {
                // Failover on: use the configured values (0 = no timeout)
                (
                    self.app_config.non_streaming_timeout as u64,
                    self.app_config.streaming_first_byte_timeout as u64,
                    self.app_config.streaming_idle_timeout as u64,
                )
            } else {
                // Failover off: no timeouts
                log::debug!(
                    "[{}] Failover disabled, timeout configs are bypassed",
                    self.tag
                );
                (0, 0, 0)
            };

        RequestForwarder::new(
            state.provider_router.clone(),
            non_streaming_timeout,
            state.status.clone(),
            state.current_providers.clone(),
            state.failover_manager.clone(),
            state.app_handle.clone(),
            self.current_provider_id.clone(),
            first_byte_timeout,
            idle_timeout,
            self.rectifier_config.clone(),
            self.optimizer_config.clone(),
            self.copilot_optimizer_config.clone(),
        )
    }

    /// Provider list (for failover)
    ///
    /// Returns the providers selected when the context was created, avoiding another select_providers() call
    pub fn get_providers(&self) -> Vec<Provider> {
        self.providers.clone()
    }

    /// Request latency (milliseconds)
    #[inline]
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Streaming timeout configuration
    ///
    /// Configuration rules:
    /// - Failover on: the configured values (0 disables the timeout check)
    /// - Failover off: 0 (timeout checks disabled)
    #[inline]
    pub fn streaming_timeout_config(&self) -> StreamingTimeoutConfig {
        if self.app_config.auto_failover_enabled {
            // Failover on: use the configured values (0 = no timeout)
            StreamingTimeoutConfig {
                first_byte_timeout: self.app_config.streaming_first_byte_timeout as u64,
                idle_timeout: self.app_config.streaming_idle_timeout as u64,
            }
        } else {
            // Failover off: disable streaming timeout checks
            StreamingTimeoutConfig {
                first_byte_timeout: 0,
                idle_timeout: 0,
            }
        }
    }
}
