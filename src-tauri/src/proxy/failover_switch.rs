//! Failover switching
//!
//! Switches the provider after a successful failover, including:
//! - deduplication (so concurrent requests don't all trigger it)
//! - tray menu update
//! - frontend event emission

use crate::database::Database;
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
            .do_switch(app_handle, app_type, provider_id, provider_name)
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
                switched = app_state
                    .proxy_service
                    .hot_switch_provider(app_type, provider_id)
                    .await
                    .map_err(AppError::Message)?
                    .logical_target_changed;

                if !switched {
                    return Ok(false);
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
