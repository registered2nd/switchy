#![allow(non_snake_case)]

use tauri::AppHandle;

/// Get the settings
#[tauri::command]
pub async fn get_settings() -> Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::get_settings_for_frontend())
}

/// Save the settings
#[tauri::command]
pub async fn save_settings(settings: crate::settings::AppSettings) -> Result<bool, String> {
    crate::settings::update_settings(settings).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Restart the app (used after app_config_dir changes)
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<bool, String> {
    log::info!("Restart requested from the settings page");
    // Restart after a short delay in the background so this call can return
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        app.restart();
    });
    Ok(true)
}

/// Get the app_config_dir override (from the Store)
#[tauri::command]
pub async fn get_app_config_dir_override(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::app_store::refresh_app_config_dir_override(&app)
        .map(|p| p.to_string_lossy().to_string()))
}

/// Set the app_config_dir override (in the Store)
#[tauri::command]
pub async fn set_app_config_dir_override(
    app: AppHandle,
    path: Option<String>,
) -> Result<bool, String> {
    crate::app_store::set_app_config_dir_to_store(&app, path.as_deref())?;
    Ok(true)
}

/// Turn launch at login on or off
#[tauri::command]
pub async fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    if enabled {
        crate::auto_launch::enable_auto_launch()
            .map_err(|e| format!("Could not turn on launch at login: {e}"))?;
    } else {
        crate::auto_launch::disable_auto_launch()
            .map_err(|e| format!("Could not turn off launch at login: {e}"))?;
    }
    Ok(true)
}

/// Whether launch at login is on
#[tauri::command]
pub async fn get_auto_launch_status() -> Result<bool, String> {
    crate::auto_launch::is_auto_launch_enabled()
        .map_err(|e| format!("Could not read the launch-at-login setting: {e}"))
}

/// Get the rectifier config
#[tauri::command]
pub async fn get_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::RectifierConfig, String> {
    state.db.get_rectifier_config().map_err(|e| e.to_string())
}

/// Set the rectifier config
#[tauri::command]
pub async fn set_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::RectifierConfig,
) -> Result<bool, String> {
    state
        .db
        .set_rectifier_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Account pool settings: quota-driven rotation, the Claude proxy path, the exit check.
#[tauri::command]
pub async fn get_account_pool_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::account_pool::AccountPoolConfig, String> {
    state
        .db
        .get_account_pool_config()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_account_pool_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::account_pool::AccountPoolConfig,
) -> Result<bool, String> {
    if !(50..=100).contains(&config.threshold_percent) {
        return Err("threshold must be between 50 and 100".to_string());
    }
    state
        .db
        .set_account_pool_config(&config)
        .map_err(|e| e.to_string())?;
    crate::proxy::codex_engine::nudge();
    Ok(true)
}

/// Quota each pooled account last reported, keyed by provider id.
#[tauri::command]
pub async fn get_account_pool_quota(
) -> Result<std::collections::HashMap<String, crate::proxy::account_pool::AccountQuota>, String> {
    Ok(crate::proxy::account_pool::quota_snapshot())
}

/// Get the optimizer config
#[tauri::command]
pub async fn get_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::OptimizerConfig, String> {
    state.db.get_optimizer_config().map_err(|e| e.to_string())
}

/// Set the optimizer config
#[tauri::command]
pub async fn set_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::OptimizerConfig,
) -> Result<bool, String> {
    // Validate cache_ttl: only allow known values
    match config.cache_ttl.as_str() {
        "5m" | "1h" => {}
        other => {
            return Err(format!(
                "Invalid cache_ttl value: '{other}'. Allowed values: '5m', '1h'"
            ))
        }
    }
    state
        .db
        .set_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Get the Copilot optimizer config
#[tauri::command]
pub async fn get_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::CopilotOptimizerConfig, String> {
    state
        .db
        .get_copilot_optimizer_config()
        .map_err(|e| e.to_string())
}

/// Set the Copilot optimizer config
#[tauri::command]
pub async fn set_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::CopilotOptimizerConfig,
) -> Result<bool, String> {
    state
        .db
        .set_copilot_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Get the log config
#[tauri::command]
pub async fn get_log_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::LogConfig, String> {
    state.db.get_log_config().map_err(|e| e.to_string())
}

/// Set the log config
#[tauri::command]
pub async fn set_log_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::LogConfig,
) -> Result<bool, String> {
    state
        .db
        .set_log_config(&config)
        .map_err(|e| e.to_string())?;
    log::set_max_level(config.to_level_filter());
    log::info!(
        "Log config updated: enabled={}, level={}",
        config.enabled,
        config.level
    );
    Ok(true)
}
