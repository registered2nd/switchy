use serde::{Deserialize, Serialize};
use std::fs;
#[cfg(unix)]
use std::io::Write;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use crate::app_config::AppType;
use crate::error::AppError;

/// Custom endpoint config (legacy; actually stored in provider.meta.custom_endpoints)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub url: String,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// Which apps are shown on the main page
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleApps {
    #[serde(default = "default_true")]
    pub claude: bool,
    #[serde(default = "default_true")]
    pub codex: bool,
    #[serde(default = "default_true")]
    pub gemini: bool,
    #[serde(default = "default_true")]
    pub kimi: bool,
    #[serde(default = "default_true")]
    pub opencode: bool,
    /// Hidden by default: OpenClaw is a gateway, not a CLI worth a tab.
    #[serde(default)]
    pub openclaw: bool,
}

impl Default for VisibleApps {
    fn default() -> Self {
        Self {
            claude: true,
            codex: true,
            gemini: true,
            kimi: true,
            opencode: true,
            openclaw: false,
        }
    }
}

impl VisibleApps {
    /// Check if the specified app is visible
    pub fn is_visible(&self, app: &AppType) -> bool {
        match app {
            AppType::Claude => self.claude,
            AppType::Codex => self.codex,
            AppType::Gemini => self.gemini,
            AppType::Kimi => self.kimi,
            AppType::OpenCode => self.opencode,
            AppType::OpenClaw => self.openclaw,
        }
    }
}

/// App settings
///
/// Holds device-level settings, saved locally in `~/.switchy/settings.json` and not synced with the database,
/// so several devices sharing synced data can each keep their own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    // ===== Device-level UI settings =====
    #[serde(default = "default_show_in_tray")]
    pub show_in_tray: bool,
    #[serde(default = "default_minimize_to_tray_on_close")]
    pub minimize_to_tray_on_close: bool,
    /// Launch at login
    #[serde(default)]
    pub launch_on_startup: bool,
    /// Silent start (no main window at startup; run in the tray only)
    #[serde(default)]
    pub silent_startup: bool,
    /// Enable the local proxy on the main page (off by default)
    #[serde(default)]
    pub enable_local_proxy: bool,
    /// User has confirmed the local proxy first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_confirmed: Option<bool>,
    /// User has confirmed the usage query first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_confirmed: Option<bool>,
    /// User has confirmed the stream check first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_check_confirmed: Option<bool>,
    /// Whether to show the failover toggle independently on the main page
    #[serde(default)]
    pub enable_failover_toggle: bool,
    /// User has confirmed the failover toggle first-run notice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failover_confirmed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    // ===== Apps shown on the main page =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_apps: Option<VisibleApps>,

    // ===== Device-level directory overrides =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_mirror_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_mirror_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kimi_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openclaw_config_dir: Option<String>,

    // ===== Current provider IDs (device-level) =====
    /// Current Claude provider ID (stored locally; takes precedence over the database is_current)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_claude: Option<String>,
    /// Current Codex provider ID (stored locally; takes precedence over the database is_current)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_codex: Option<String>,
    /// Current Gemini provider ID (stored locally; takes precedence over the database is_current)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_gemini: Option<String>,
    /// Current Kimi provider ID (stored locally; takes precedence over the database is_current)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_kimi: Option<String>,
    /// Current OpenCode provider ID (stored locally; may be meaningless for OpenCode but kept for a uniform structure)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_opencode: Option<String>,
    /// Current OpenClaw provider ID (stored locally; may be meaningless for OpenClaw but kept for a uniform structure)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_provider_openclaw: Option<String>,

    // ===== Backup policy =====
    /// Auto-backup interval in hours (default 24, 0 = disabled)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_interval_hours: Option<u32>,
    /// Maximum number of backup files to retain (default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_retain_count: Option<u32>,

    // ===== Terminal =====
    /// Preferred terminal app (optional; defaults to the system terminal)
    /// - macOS: "terminal" | "iterm2" | "warp" | "alacritty" | "kitty" | "ghostty"
    /// - Windows: "cmd" | "powershell" | "wt" (Windows Terminal)
    /// - Linux: "gnome-terminal" | "konsole" | "xfce4-terminal" | "alacritty" | "kitty" | "ghostty"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_terminal: Option<String>,
}

fn default_show_in_tray() -> bool {
    true
}

fn default_minimize_to_tray_on_close() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            show_in_tray: true,
            minimize_to_tray_on_close: true,
            launch_on_startup: false,
            silent_startup: false,
            enable_local_proxy: false,
            proxy_confirmed: None,
            usage_confirmed: None,
            stream_check_confirmed: None,
            enable_failover_toggle: false,
            failover_confirmed: None,
            language: None,
            visible_apps: None,
            claude_config_dir: None,
            claude_mirror_config_dir: None,
            codex_config_dir: None,
            codex_mirror_config_dir: None,
            gemini_config_dir: None,
            kimi_config_dir: None,
            opencode_config_dir: None,
            openclaw_config_dir: None,
            current_provider_claude: None,
            current_provider_codex: None,
            current_provider_gemini: None,
            current_provider_kimi: None,
            current_provider_opencode: None,
            current_provider_openclaw: None,
            backup_interval_hours: None,
            backup_retain_count: None,
            preferred_terminal: None,
        }
    }
}

impl AppSettings {
    fn settings_path() -> Option<PathBuf> {
        // settings.json is kept for migrating old versions and for running without a database
        Some(
            crate::config::get_home_dir()
                .join(crate::paths::APP_DIR)
                .join("settings.json"),
        )
    }

    fn normalize_paths(&mut self) {
        self.claude_config_dir = self
            .claude_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.claude_mirror_config_dir = self
            .claude_mirror_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.codex_config_dir = self
            .codex_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.codex_mirror_config_dir = self
            .codex_mirror_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.gemini_config_dir = self
            .gemini_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.kimi_config_dir = self
            .kimi_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.opencode_config_dir = self
            .opencode_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.openclaw_config_dir = self
            .openclaw_config_dir
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        self.language = self
            .language
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| matches!(*s, "en" | "zh" | "ja"))
            .map(|s| s.to_string());
    }

    fn load_from_file() -> Self {
        let Some(path) = Self::settings_path() else {
            return Self::default();
        };
        if let Ok(content) = fs::read_to_string(&path) {
            match serde_json::from_str::<AppSettings>(&content) {
                Ok(mut settings) => {
                    settings.normalize_paths();
                    settings
                }
                Err(err) => {
                    log::warn!(
                        "Failed to parse the settings file; using defaults. Path: {}, error: {}",
                        path.display(),
                        err
                    );
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }
}

fn save_settings_file(settings: &AppSettings) -> Result<(), AppError> {
    let mut normalized = settings.clone();
    normalized.normalize_paths();
    let Some(path) = AppSettings::settings_path() else {
        return Err(AppError::Config(
            "Could not get the user home directory".to_string(),
        ));
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| AppError::io(&path, e))?;
        file.write_all(json.as_bytes())
            .map_err(|e| AppError::io(&path, e))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, json).map_err(|e| AppError::io(&path, e))?;
    }

    Ok(())
}

static SETTINGS_STORE: OnceLock<RwLock<AppSettings>> = OnceLock::new();

fn settings_store() -> &'static RwLock<AppSettings> {
    SETTINGS_STORE.get_or_init(|| RwLock::new(AppSettings::load_from_file()))
}

fn resolve_override_path(raw: &str) -> PathBuf {
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

pub fn get_settings() -> AppSettings {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("Settings lock poisoned; using the recovered value: {e}");
            e.into_inner()
        })
        .clone()
}

pub fn get_settings_for_frontend() -> AppSettings {
    get_settings()
}

pub fn update_settings(mut new_settings: AppSettings) -> Result<(), AppError> {
    new_settings.normalize_paths();
    save_settings_file(&new_settings)?;

    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("Settings lock poisoned; using the recovered value: {e}");
        e.into_inner()
    });
    *guard = new_settings;
    Ok(())
}

fn mutate_settings<F>(mutator: F) -> Result<(), AppError>
where
    F: FnOnce(&mut AppSettings),
{
    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("Settings lock poisoned; using the recovered value: {e}");
        e.into_inner()
    });
    let mut next = guard.clone();
    mutator(&mut next);
    next.normalize_paths();
    save_settings_file(&next)?;
    *guard = next;
    Ok(())
}

/// Reload settings from the file into the in-memory cache
/// Used after a config import and similar, to keep the cache in step with the file
pub fn reload_settings() -> Result<(), AppError> {
    let fresh_settings = AppSettings::load_from_file();
    let mut guard = settings_store().write().unwrap_or_else(|e| {
        log::warn!("Settings lock poisoned; using the recovered value: {e}");
        e.into_inner()
    });
    *guard = fresh_settings;
    Ok(())
}

pub fn get_claude_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .claude_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

#[cfg(test)]
pub fn set_claude_mirror_config_dir(value: Option<PathBuf>) -> Result<(), AppError> {
    mutate_settings(|s| {
        s.claude_mirror_config_dir = value.map(|p| p.to_string_lossy().to_string());
    })
}

pub fn get_claude_mirror_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    if let Some(p) = settings.claude_mirror_config_dir.as_ref() {
        return Some(resolve_override_path(p));
    }
    drop(settings);
    // The auto-detected WSL default is a machine-global side-channel that the
    // `SWITCHY_TEST_HOME` redirect cannot sandbox: on a developer machine that
    // happens to have WSL, every "no mirror configured" test would quietly
    // become a mirror test and stop checking what it was written to check.
    // Tests that want a mirror set one explicitly.
    if std::env::var_os(crate::paths::ENV_TEST_HOME).is_some() {
        return None;
    }
    static DEFAULT_MIRROR: OnceLock<Option<String>> = OnceLock::new();
    DEFAULT_MIRROR
        .get_or_init(crate::commands::config::build_default_claude_mirror_dir)
        .as_ref()
        .map(PathBuf::from)
}

/// The second `~/.codex` a switch keeps in step (typically WSL). Same
/// auto-detection and test-sandbox rules as `get_claude_mirror_override_dir`.
pub fn get_codex_mirror_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    if let Some(p) = settings.codex_mirror_config_dir.as_ref() {
        return Some(resolve_override_path(p));
    }
    drop(settings);
    if std::env::var_os(crate::paths::ENV_TEST_HOME).is_some() {
        return None;
    }
    static DEFAULT_MIRROR: OnceLock<Option<String>> = OnceLock::new();
    DEFAULT_MIRROR
        .get_or_init(crate::commands::config::build_default_codex_mirror_dir)
        .as_ref()
        .map(PathBuf::from)
}

pub fn get_codex_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .codex_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_kimi_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .kimi_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_gemini_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .gemini_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_opencode_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .opencode_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

pub fn get_openclaw_override_dir() -> Option<PathBuf> {
    let settings = settings_store().read().ok()?;
    settings
        .openclaw_config_dir
        .as_ref()
        .map(|p| resolve_override_path(p))
}

// ===== Current provider =====

/// Get the current provider ID for an app type (from local settings)
///
/// This is a device-level setting and is not synced with the database.
/// If it is not set locally, callers should fall back to the database `is_current` field.
pub fn get_current_provider(app_type: &AppType) -> Option<String> {
    let settings = settings_store().read().ok()?;
    match app_type {
        AppType::Claude => settings.current_provider_claude.clone(),
        AppType::Codex => settings.current_provider_codex.clone(),
        AppType::Gemini => settings.current_provider_gemini.clone(),
        AppType::Kimi => settings.current_provider_kimi.clone(),
        AppType::OpenCode => settings.current_provider_opencode.clone(),
        AppType::OpenClaw => settings.current_provider_openclaw.clone(),
    }
}

/// Set the current provider ID for an app type (saved to local settings)
///
/// This is a device-level setting and is not synced with the database.
/// Passing `None` clears the current provider.
pub fn set_current_provider(app_type: &AppType, id: Option<&str>) -> Result<(), AppError> {
    let id_owned = id.map(|s| s.to_string());
    mutate_settings(|settings| match app_type {
        AppType::Claude => settings.current_provider_claude = id_owned.clone(),
        AppType::Codex => settings.current_provider_codex = id_owned.clone(),
        AppType::Gemini => settings.current_provider_gemini = id_owned.clone(),
        AppType::Kimi => settings.current_provider_kimi = id_owned.clone(),
        AppType::OpenCode => settings.current_provider_opencode = id_owned.clone(),
        AppType::OpenClaw => settings.current_provider_openclaw = id_owned.clone(),
    })
}

/// Get the effective current provider ID (checked to exist)
///
/// Logic:
/// 1. read the current provider ID from local settings
/// 2. check that the ID exists in the database
/// 3. if not, clear it from local settings and fall back to the database is_current
///
/// So the returned ID is always valid (exists in the database).
/// When data is synced across devices, an import can invalidate the local ID; this repairs it.
pub fn get_effective_current_provider(
    db: &crate::database::Database,
    app_type: &AppType,
) -> Result<Option<String>, AppError> {
    // 1. Read from local settings
    if let Some(local_id) = get_current_provider(app_type) {
        // 2. Check the ID exists in the database
        let providers = db.get_all_providers(app_type.as_str())?;
        if providers.contains_key(&local_id) {
            // It exists; return it
            return Ok(Some(local_id));
        }

        // 3. It does not exist; clear local settings
        log::warn!(
            "Provider {} ({}) in local settings does not exist in the database; clearing it and falling back to the database",
            local_id,
            app_type.as_str()
        );
        let _ = set_current_provider(app_type, None);
    }

    // Fall back to the database is_current
    db.get_current_provider(app_type.as_str())
}

// ===== Backup policy =====

/// Get the effective auto-backup interval in hours (default 24)
pub fn effective_backup_interval_hours() -> u32 {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("Settings lock poisoned; using the recovered value: {e}");
            e.into_inner()
        })
        .backup_interval_hours
        .unwrap_or(24)
}

/// Get the effective backup retain count (default 10, minimum 1)
pub fn effective_backup_retain_count() -> usize {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("Settings lock poisoned; using the recovered value: {e}");
            e.into_inner()
        })
        .backup_retain_count
        .map(|n| (n as usize).max(1))
        .unwrap_or(10)
}

// ===== Terminal =====

/// Get the preferred terminal app
pub fn get_preferred_terminal() -> Option<String> {
    settings_store()
        .read()
        .unwrap_or_else(|e| {
            log::warn!("Settings lock poisoned; using the recovered value: {e}");
            e.into_inner()
        })
        .preferred_terminal
        .clone()
}
