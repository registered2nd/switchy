//! Failover switching
//!
//! Switches the provider after a successful failover, including:
//! - deduplication (so concurrent requests don't all trigger it)
//! - tray menu update
//! - frontend event emission

use crate::database::Database;
use crate::database::SwitchReason;
use crate::error::AppError;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

/// Failover switch manager
///
/// Switches the provider after a successful failover so the UI shows the provider actually in use.
#[derive(Clone)]
pub struct FailoverSwitchManager {
    /// Switches in progress (key = "app_type:provider_id")
    pending_switches: Arc<RwLock<HashSet<String>>>,
    db: Arc<Database>,
}

impl FailoverSwitchManager {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            pending_switches: Arc::new(RwLock::new(HashSet::new())),
            db,
        }
    }

    /// Attempts a failover switch
    ///
    /// Skips if the same switch is already in progress; otherwise performs it.
    /// `passed_over` says why the previous provider was skipped; the switch is
    /// recorded in the account switch history with it.
    ///
    /// # Returns
    /// - `Ok(true)` - the switch was performed
    /// - `Ok(false)` - the switch was already in progress; skipped
    /// - `Err(e)` - an error occurred during the switch
    pub async fn try_switch(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        app_type: &str,
        provider_id: &str,
        provider_name: &str,
        passed_over: Option<(SwitchReason, String)>,
    ) -> Result<bool, AppError> {
        let switch_key = format!("{app_type}:{provider_id}");

        // Deduplicate: skip if the same switch is already in progress
        {
            let mut pending = self.pending_switches.write().await;
            if pending.contains(&switch_key) {
                log::debug!(
                    "[Failover] Switch already in progress, skipping: {app_type} -> {provider_id}"
                );
                return Ok(false);
            }
            pending.insert(switch_key.clone());
        }

        // Perform the switch (always clearing the pending marker afterwards)
        let result = self
            .do_switch(
                app_handle,
                app_type,
                provider_id,
                provider_name,
                passed_over,
            )
            .await;

        // Clear the pending marker
        {
            let mut pending = self.pending_switches.write().await;
            pending.remove(&switch_key);
        }

        result
    }

    async fn do_switch(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        app_type: &str,
        provider_id: &str,
        provider_name: &str,
        passed_over: Option<(SwitchReason, String)>,
    ) -> Result<bool, AppError> {
        // Check whether the proxy has taken over this app (enabled=true)
        // Only taken-over apps may perform a failover switch
        let app_enabled = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(config) => config.enabled,
            Err(e) => {
                log::warn!("[FO-002] Cannot read {app_type} config: {e}; skipping switch");
                return Ok(false);
            }
        };

        if !app_enabled {
            log::debug!("[Failover] {app_type} is not proxied; skipping switch");
            return Ok(false);
        }

        log::info!("[FO-001] Switch: {app_type} → {provider_name}");

        let mut switched = false;

        if let Some(app) = app_handle {
            if let Some(app_state) = app.try_state::<crate::store::AppState>() {
                let previous = app_state.db.get_current_provider(app_type).ok().flatten();
                switched = app_state
                    .proxy_service
                    .hot_switch_provider(app_type, provider_id)
                    .await
                    .map_err(AppError::Message)?
                    .logical_target_changed;

                if !switched {
                    return Ok(false);
                }

                // Claude Code's saved login follows the account the pool moved to.
                if app_type == "claude" {
                    if let Ok(Some(provider)) =
                        app_state.db.get_provider_by_id(provider_id, app_type)
                    {
                        for warning in crate::services::provider::ProviderService::swap_claude_login(
                            app_state.inner(),
                            &provider,
                        ) {
                            log::warn!("[Failover] Claude login swap: {warning}");
                        }
                    }
                }

                let (reason, detail) =
                    passed_over.unwrap_or((SwitchReason::Failover, String::new()));
                if let Err(e) = self.db.record_account_switch(
                    app_type,
                    previous.as_deref(),
                    provider_id,
                    reason,
                    Some(detail.as_str()).filter(|d| !d.is_empty()),
                ) {
                    log::warn!("[Failover] Could not record the switch: {e}");
                }

                if let Ok(new_menu) = crate::tray::create_tray_menu(app, app_state.inner()) {
                    if let Some(tray) = app.tray_by_id("main") {
                        if let Err(e) = tray.set_menu(Some(new_menu)) {
                            log::error!("[Failover] Failed to update tray menu: {e}");
                        }
                    }
                }
            }

            // Emit event to the frontend
            let event_data = serde_json::json!({
                "appType": app_type,
                "providerId": provider_id,
                "source": "failover"  // marks the source as failover
            });
            if let Err(e) = app.emit("provider-switched", event_data) {
                log::error!("[Failover] Failed to emit event: {e}");
            }
        }

        Ok(switched)
    }
}
