use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use indexmap::IndexMap;
use std::collections::HashMap;
use switchy_lib::{
    update_settings, AppSettings, AppState, AppType, Database, Provider, ProxyService,
};

/// Set up an isolated HOME directory for tests so real user data is not touched.
pub fn ensure_test_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let base = std::env::temp_dir().join("switchy-test-home");
        if base.exists() {
            let _ = std::fs::remove_dir_all(&base);
        }
        std::fs::create_dir_all(&base).expect("create test home");
        // On Windows, `dirs::home_dir()` ignores HOME/USERPROFILE (it uses the Known Folder API),
        // so SWITCHY_TEST_HOME overrides it explicitly to keep tests out of the real user directory.
        std::env::set_var("SWITCHY_TEST_HOME", &base);
        std::env::set_var("HOME", &base);
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &base);
        base
    })
    .as_path()
}

/// Remove config files and caches generated in the test directory.
pub fn reset_test_fs() {
    let home = ensure_test_home();
    for sub in [
        ".claude",
        ".codex",
        ".switchy",
        ".gemini",
        ".config",
        ".openclaw",
        ".kimi-code",
    ] {
        let path = home.join(sub);
        if path.exists() {
            if let Err(err) = std::fs::remove_dir_all(&path) {
                eprintln!("failed to clean {}: {}", path.display(), err);
            }
        }
    }
    let claude_json = home.join(".claude.json");
    if claude_json.exists() {
        let _ = std::fs::remove_file(&claude_json);
    }

    // Reset the in-memory settings cache so the test environment is not affected by the previous call
    let _ = update_settings(AppSettings::default());
}

/// Global mutex so concurrent tests do not write to the same HOME directory.
pub fn test_mutex() -> &'static Mutex<()> {
    static MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    MUTEX.get_or_init(|| Mutex::new(()))
}

/// Create a test AppState with an empty database
#[allow(dead_code)]
pub fn create_test_state() -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    let proxy_service = ProxyService::new(db.clone());
    Ok(AppState { db, proxy_service })
}

/// Providers per app for a test database: each app's providers in order and
/// the current one.
#[derive(Default)]
#[allow(dead_code)]
pub struct TestManager {
    pub providers: IndexMap<String, Provider>,
    pub current: String,
}

#[derive(Default)]
#[allow(dead_code)]
pub struct TestConfig {
    managers: HashMap<String, TestManager>,
}

#[allow(dead_code)]
impl TestConfig {
    pub fn get_manager_mut(&mut self, app: &AppType) -> Option<&mut TestManager> {
        Some(self.managers.entry(app.as_str().to_string()).or_default())
    }

    pub fn get_manager(&self, app: &AppType) -> Option<&TestManager> {
        self.managers.get(app.as_str())
    }
}

/// A test `AppState` whose database holds `config`'s providers.
#[allow(dead_code)]
pub fn create_test_state_with_config(
    config: &TestConfig,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    for (app, manager) in &config.managers {
        for (index, provider) in manager.providers.values().enumerate() {
            let mut provider = provider.clone();
            provider.sort_index.get_or_insert(index);
            db.save_provider(app, &provider)?;
        }
        if !manager.current.is_empty() {
            db.set_current_provider(app, &manager.current)?;
        }
    }
    let proxy_service = ProxyService::new(db.clone());
    Ok(AppState { db, proxy_service })
}
