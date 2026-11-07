use serde_json::Value;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use tauri_plugin_store::StoreExt;

use crate::error::AppError;

/// Key name in the Store
const STORE_KEY_APP_CONFIG_DIR: &str = "app_config_dir_override";

/// Cached app_config_dir override path, so the AppHandle need not be stored
static APP_CONFIG_DIR_OVERRIDE: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn override_cache() -> &'static RwLock<Option<PathBuf>> {
    APP_CONFIG_DIR_OVERRIDE.get_or_init(|| RwLock::new(None))
}

fn update_cached_override(value: Option<PathBuf>) {
    if let Ok(mut guard) = override_cache().write() {
        *guard = value;
    }
}

/// Get the cached app_config_dir override path
pub fn get_app_config_dir_override() -> Option<PathBuf> {
    override_cache().read().ok()?.clone()
}

fn read_override_from_store(app: &tauri::AppHandle) -> Option<PathBuf> {
    let store = match app.store_builder("app_paths.json").build() {
        Ok(store) => store,
        Err(e) => {
            log::warn!("Failed to create Store: {e}");
            return None;
        }
    };

    match store.get(STORE_KEY_APP_CONFIG_DIR) {
        Some(Value::String(path_str)) => {
            let path_str = path_str.trim();
            if path_str.is_empty() {
                return None;
            }

            let path = resolve_path(path_str);

            if !path.exists() {
                log::warn!(
                    "app_config_dir configured in the Store does not exist: {path:?}\n\
                     Using the default path."
                );
                return None;
            }

            log::info!("Using app_config_dir from the Store: {path:?}");
            Some(path)
        }
        Some(_) => {
            log::warn!(
                "{STORE_KEY_APP_CONFIG_DIR} in the Store has the wrong type; expected a string"
            );
            None
        }
        None => None,
    }
}

/// Reload the app_config_dir override from the Store and update the cache
pub fn refresh_app_config_dir_override(app: &tauri::AppHandle) -> Option<PathBuf> {
    let value = read_override_from_store(app);
    update_cached_override(value.clone());
    value
}

/// Write app_config_dir to the Tauri Store
pub fn set_app_config_dir_to_store(
    app: &tauri::AppHandle,
    path: Option<&str>,
) -> Result<(), AppError> {
    let store = app
        .store_builder("app_paths.json")
        .build()
        .map_err(|e| AppError::Message(format!("Failed to create Store: {e}")))?;

    match path {
        Some(p) => {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                store.set(STORE_KEY_APP_CONFIG_DIR, Value::String(trimmed.to_string()));
                log::info!("Wrote app_config_dir to the Store: {trimmed}");
            } else {
                store.delete(STORE_KEY_APP_CONFIG_DIR);
                log::info!("Removed app_config_dir from the Store");
            }
        }
        None => {
            store.delete(STORE_KEY_APP_CONFIG_DIR);
            log::info!("Removed app_config_dir from the Store");
        }
    }

    store
        .save()
        .map_err(|e| AppError::Message(format!("Failed to save Store: {e}")))?;

    refresh_app_config_dir_override(app);
    Ok(())
}

/// Resolve a path, expanding a leading ~
fn resolve_path(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if let Some(stripped) = raw.strip_prefix("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw)
}

/// Migrate app_config_dir from the old settings.json to the Store
pub fn migrate_app_config_dir_from_settings(app: &tauri::AppHandle) -> Result<(), AppError> {
    // app_config_dir is no longer in settings.json; this function is kept but performs no migration.
    // Users who set app_config_dir in an old version must configure it again in the Store.
    log::info!("app_config_dir migration has been removed; configure it again in Settings");

    let _ = refresh_app_config_dir_override(app);
    Ok(())
}
