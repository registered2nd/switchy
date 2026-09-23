//! Failover queue commands
//!
//! Manages the failover queue in proxy mode (the in_failover_queue column of the providers table)

use crate::database::FailoverQueueItem;
use crate::provider::Provider;
use crate::store::AppState;
use std::str::FromStr;
use tauri::Emitter;

/// Get the failover queue
#[tauri::command]
pub async fn get_failover_queue(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<Vec<FailoverQueueItem>, String> {
    state
        .db
        .get_failover_queue(&app_type)
        .map_err(|e| e.to_string())
}

/// Get the providers that can be added to the failover queue (those not already in it)
#[tauri::command]
pub async fn get_available_providers_for_failover(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<Vec<Provider>, String> {
    state
        .db
        .get_available_providers_for_failover(&app_type)
        .map_err(|e| e.to_string())
}

/// Add a provider to the failover queue
#[tauri::command]
pub async fn add_to_failover_queue(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: String,
) -> Result<(), String> {
    state
        .db
        .add_to_failover_queue(&app_type, &provider_id)
        .map_err(|e| e.to_string())
}

/// Remove a provider from the failover queue
#[tauri::command]
pub async fn remove_from_failover_queue(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: String,
) -> Result<(), String> {
    state
        .db
        .remove_from_failover_queue(&app_type, &provider_id)
        .map_err(|e| e.to_string())
}

/// Get the auto-failover switch for an app (read from the proxy_config table)
#[tauri::command]
pub async fn get_auto_failover_enabled(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<bool, String> {
    state
        .db
        .get_proxy_config_for_app(&app_type)
        .await
        .map(|config| config.auto_failover_enabled)
        .map_err(|e| e.to_string())
}

/// Set the auto-failover switch for an app (written to the proxy_config table)
///
/// Turning failover off does not clear the queue; it is kept for the next time failover is enabled
#[tauri::command]
pub async fn set_auto_failover_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    app_type: String,
    enabled: bool,
) -> Result<(), String> {
    log::info!(
        "[Failover] Setting auto_failover_enabled: app_type='{app_type}', enabled={enabled}"
    );

    // Strong consistency: enabling failover switches to queue P1 immediately (and makes sure the queue is not empty)
    //
    // Notes:
    // - Switch to P1 only when enabled=true
    // - If the queue is empty, add the current provider as P1, so the user is not deadlocked in the UI (unable to enable before adding to the queue)
    let p1_provider_id = if enabled {
        let mut queue = state
            .db
            .get_failover_queue(&app_type)
            .map_err(|e| e.to_string())?;

        if queue.is_empty() {
            let app_enum = crate::app_config::AppType::from_str(&app_type)
                .map_err(|_| format!("Invalid app type: {app_type}"))?;

            let current_id = crate::settings::get_effective_current_provider(&state.db, &app_enum)
                .map_err(|e| e.to_string())?;

            let Some(current_id) = current_id else {
                return Err("The failover queue is empty and no current provider is set; cannot enable failover".to_string());
            };

            state
                .db
                .add_to_failover_queue(&app_type, &current_id)
                .map_err(|e| e.to_string())?;

            queue = state
                .db
                .get_failover_queue(&app_type)
                .map_err(|e| e.to_string())?;
        }

        queue
            .first()
            .map(|item| item.provider_id.clone())
            .ok_or_else(|| "The failover queue is empty; cannot enable failover".to_string())?
    } else {
        String::new()
    };

    // Read the current config
    let mut config = state
        .db
        .get_proxy_config_for_app(&app_type)
        .await
        .map_err(|e| e.to_string())?;

    // Update the auto_failover_enabled field
    config.auto_failover_enabled = enabled;

    // Write back to the database
    state
        .db
        .update_proxy_config_for_app(config)
        .await
        .map_err(|e| e.to_string())?;

    // After enabling, switch to P1 immediately: update is_current + local settings + the live backup (in takeover mode)
    if enabled {
        state
            .proxy_service
            .switch_proxy_target(&app_type, &p1_provider_id)
            .await?;

        // Emit the provider-switched event (so the frontend refreshes the current provider)
        let event_data = serde_json::json!({
            "appType": app_type,
            "providerId": p1_provider_id,
            "source": "failoverEnabled"
        });
        let _ = app.emit("provider-switched", event_data);
    }

    // Refresh the tray menu to keep state in sync
    if let Ok(new_menu) = crate::tray::create_tray_menu(&app, &state) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(new_menu));
        }
    }

    Ok(())
}
