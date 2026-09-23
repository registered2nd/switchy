use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// Get the user's home directory, with fallback and logging
///
/// ## Windows notes
///
/// - On Windows, `dirs::home_dir()` uses `SHGetKnownFolderPath(FOLDERID_Profile)`,
///   which returns the real user directory (like `C:\\Users\\Alice`).
/// - Do not use the `HOME` environment variable directly: third-party tools such as Git/Cygwin/MSYS
///   may inject it and it need not equal the user directory, so the `.switchy/switchy.db` path could move
///   and "look like data loss".
///
/// ## Test isolation
///
/// So that Windows CI and local tests reliably stay away from real user data, `SWITCHY_TEST_HOME`
/// explicitly overrides the home dir (for testing/debugging only).
pub fn get_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var(crate::paths::ENV_TEST_HOME) {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::home_dir().unwrap_or_else(|| {
        log::warn!("Could not get the user home directory; falling back to the current directory");
        PathBuf::from(".")
    })
}

/// Get the Claude Code config directory path
pub fn get_claude_config_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_claude_override_dir() {
        return custom;
    }

    get_home_dir().join(".claude")
}

/// Default Claude MCP config file path (~/.claude.json)
pub fn get_default_claude_mcp_path() -> PathBuf {
    get_home_dir().join(".claude.json")
}

/// Derive the `.claude.json` path from a Claude config *directory*.
///
/// The file is a **sibling** of the config directory, not a file inside it: the `~/.claude` directory
/// maps to `~/.claude.json`. A `.claude.json` inside the directory is a separate, independent
/// config, not the one the local CLI reads and writes; picking a file by whether it exists writes to the wrong one.
pub fn claude_config_json_for_dir(dir: &Path) -> Option<PathBuf> {
    let file_name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())?
        .trim()
        .to_string();
    if file_name.is_empty() {
        return None;
    }
    let parent = dir.parent().unwrap_or_else(|| Path::new(""));
    Some(parent.join(format!("{file_name}.json")))
}

/// Get the path of Claude Code's main config file `.claude.json`.
///
/// Every caller that reads or writes root-level fields such as `oauthAccount` / `mcpServers` must go through here;
/// otherwise modules pick different files and cannot see each other's writes.
pub fn get_claude_config_json_path() -> PathBuf {
    if let Some(custom_dir) = crate::settings::get_claude_override_dir() {
        if let Some(path) = claude_config_json_for_dir(&custom_dir) {
            return path;
        }
    }
    get_default_claude_mcp_path()
}

/// Get the Claude MCP config file path; with a directory override it sits next to the override directory
pub fn get_claude_mcp_path() -> PathBuf {
    get_claude_config_json_path()
}

/// Get the Claude Code main settings file path
pub fn get_claude_settings_path() -> PathBuf {
    let dir = get_claude_config_dir();
    let settings = dir.join("settings.json");
    if settings.exists() {
        return settings;
    }
    // Legacy name: keep using the old file if it exists
    let legacy = dir.join("claude.json");
    if legacy.exists() {
        return legacy;
    }
    // Default for new setups: the standard settings.json (claude.json is no longer created)
    settings
}

/// Get the app config directory path (~/.switchy)
pub fn get_app_config_dir() -> PathBuf {
    if let Some(custom) = crate::app_store::get_app_config_dir_override() {
        return custom;
    }

    get_home_dir().join(crate::paths::APP_DIR)
}

/// Get the app config file path
pub fn get_app_config_path() -> PathBuf {
    get_app_config_dir().join("config.json")
}

/// Sanitise a provider name so it is safe as a file name
#[allow(dead_code)]
pub fn sanitize_provider_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

/// Get a provider config file path
#[allow(dead_code)]
pub fn get_provider_config_path(provider_id: &str, provider_name: Option<&str>) -> PathBuf {
    let base_name = provider_name
        .map(sanitize_provider_name)
        .unwrap_or_else(|| sanitize_provider_name(provider_id));

    get_claude_config_dir().join(format!("settings-{base_name}.json"))
}

/// Read a JSON config file
pub fn read_json_file<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, AppError> {
    if !path.exists() {
        return Err(AppError::Config(format!(
            "File does not exist: {}",
            path.display()
        )));
    }

    let content = fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;

    serde_json::from_str(&content).map_err(|e| AppError::json(path, e))
}

/// Write a JSON config file
pub fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    // Make sure the directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let json =
        serde_json::to_string_pretty(data).map_err(|e| AppError::JsonSerialize { source: e })?;

    atomic_write(path, json.as_bytes())
}

/// Atomically write a text file (for TOML/plain text)
pub fn write_text_file(path: &Path, data: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    atomic_write(path, data.as_bytes())
}

/// Atomic write: write a temp file, then rename it over the target, so no half-written state is visible
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("Invalid path".to_string()))?;
    let mut tmp = parent.to_path_buf();
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("Invalid file name".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    tmp.push(format!("{file_name}.tmp.{ts}"));

    {
        let mut f = fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
        f.write_all(data).map_err(|e| AppError::io(&tmp, e))?;
        f.flush().map_err(|e| AppError::io(&tmp, e))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let perm = meta.permissions().mode();
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(perm));
        }
    }

    #[cfg(windows)]
    {
        // On Windows, std's fs::rename uses MoveFileExW + MOVEFILE_REPLACE_EXISTING,
        // which replaces an existing target directly. Do not remove_file first: the file would briefly vanish,
        // and a process watching it (such as a running Claude Code) would see delete + create instead of one modify.
        fs::rename(&tmp, path).map_err(|e| AppError::IoContext {
            context: format!(
                "Atomic replace failed: {} -> {}",
                tmp.display(),
                path.display()
            ),
            source: e,
        })?;
    }

    #[cfg(not(windows))]
    {
        fs::rename(&tmp, path).map_err(|e| AppError::IoContext {
            context: format!(
                "Atomic replace failed: {} -> {}",
                tmp.display(),
                path.display()
            ),
            source: e,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_config_json_is_the_sibling_of_a_hidden_config_dir() {
        let override_dir = PathBuf::from("/tmp/profile/.claude");
        let derived =
            claude_config_json_for_dir(&override_dir).expect("should derive path for nested dir");
        assert_eq!(derived, PathBuf::from("/tmp/profile/.claude.json"));
    }

    #[test]
    fn claude_config_json_is_the_sibling_of_a_plain_config_dir() {
        let override_dir = PathBuf::from("/data/claude-config");
        let derived =
            claude_config_json_for_dir(&override_dir).expect("should derive path for standard dir");
        assert_eq!(derived, PathBuf::from("/data/claude-config.json"));
    }

    #[test]
    fn claude_config_json_supports_relative_rootless_dir() {
        let override_dir = PathBuf::from("claude");
        let derived = claude_config_json_for_dir(&override_dir)
            .expect("should derive path for single segment");
        assert_eq!(derived, PathBuf::from("claude.json"));
    }

    #[test]
    fn claude_config_json_for_root_like_dir_returns_none() {
        let override_dir = PathBuf::from("/");
        assert!(claude_config_json_for_dir(&override_dir).is_none());
    }
}

/// Delete a file
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| AppError::io(path, e))?;
    }
    Ok(())
}

/// Check the Claude Code config status
#[derive(Serialize, Deserialize)]
pub struct ConfigStatus {
    pub exists: bool,
    pub path: String,
}

/// Get the Claude Code config status
pub fn get_claude_config_status() -> ConfigStatus {
    let path = get_claude_settings_path();
    ConfigStatus {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
    }
}
