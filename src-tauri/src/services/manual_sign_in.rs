//! Own only Claude's browser override and Cursor CLI's `NO_OPEN_BROWSER`.
//! Keep the prior value for turning it off.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::settings::AppSettings;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserBackup {
    had_browser: bool,
    previous_browser: Value,
    had_env: bool,
    had_settings: bool,
    installed_browser: String,
}

fn read_backup(path: &Path) -> Result<BrowserBackup, AppError> {
    serde_json::from_value(crate::config::read_json_file(path)?)
        .map_err(|error| AppError::json(path, error))
}

fn claude_dirs(settings: &AppSettings) -> Vec<(PathBuf, bool)> {
    let local = settings
        .claude_config_dir
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(crate::settings::resolve_override_path)
        .unwrap_or_else(|| crate::config::get_home_dir().join(".claude"));
    let mut dirs = vec![(local, cfg!(windows))];
    if let Some(raw) = settings
        .claude_mirror_config_dir
        .as_deref()
        .filter(|path| !path.trim().is_empty())
    {
        let path = crate::settings::reachable_wsl_path(crate::settings::resolve_override_path(raw));
        let text = path.to_string_lossy().replace('/', "\\");
        let windows = cfg!(windows)
            && !text.starts_with("\\\\wsl$\\")
            && !text.starts_with("\\\\wsl.localhost\\");
        if !dirs.iter().any(|(dir, _)| *dir == path) {
            dirs.push((path, windows));
        }
    }
    dirs
}

fn native_browser() -> Result<String, AppError> {
    #[cfg(windows)]
    {
        let path = crate::config::get_app_config_dir()
            .join("manual_sign_in")
            .join("no_browser.exe");
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/no_browser.exe"));
        if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            crate::config::atomic_write(&path, bytes)?;
        }
        Ok(path.to_string_lossy().into_owned())
    }
    #[cfg(not(windows))]
    {
        Ok("/usr/bin/true".into())
    }
}

/// Called on startup and after a settings change. The installed shell marker
/// and Claude override keep working when Switchy is not running.
pub fn apply(settings: &AppSettings) -> Result<(), AppError> {
    let browser = if settings.manual_sign_in {
        native_browser()?
    } else {
        String::new()
    };
    for (dir, windows) in claude_dirs(settings) {
        configure_claude(
            &dir,
            settings.manual_sign_in,
            if windows { &browser } else { "/usr/bin/true" },
        )?;
    }
    configure_cursor(settings.manual_sign_in)?;
    crate::proxy::codex_engine::configure_manual_sign_in(settings)
}

/// What `NO_OPEN_BROWSER` held in the user environment before Switchy set it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnvBackup {
    previous: Option<String>,
}

/// `cursor-agent login` prints its sign-in link instead of opening a browser
/// while `NO_OPEN_BROWSER` is set, which Switchy does in the user environment
/// so it holds in every terminal, with Switchy closed too.
fn configure_cursor(enabled: bool) -> Result<(), AppError> {
    if cfg!(test) || std::env::var_os(crate::paths::ENV_TEST_HOME).is_some() {
        return Ok(());
    }
    configure_cursor_env(enabled)
}

#[cfg(windows)]
fn configure_cursor_env(enabled: bool) -> Result<(), AppError> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    const NAME: &str = "NO_OPEN_BROWSER";
    const VALUE: &str = "1";
    let backup_path = crate::config::get_app_config_dir()
        .join("manual_sign_in")
        .join("cursor_env.json");
    let env = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Environment")
        .map_err(|e| AppError::Message(format!("Could not open the user environment: {e}")))?
        .0;
    let current: Option<String> = env.get_value(NAME).ok();
    let changed = if enabled {
        if !backup_path.exists() {
            crate::config::write_json_file(
                &backup_path,
                &EnvBackup {
                    previous: current.clone(),
                },
            )?;
        }
        if current.as_deref() != Some(VALUE) {
            env.set_value(NAME, &VALUE)
                .map_err(|e| AppError::Message(format!("Could not set {NAME}: {e}")))?;
            true
        } else {
            false
        }
    } else {
        if !backup_path.exists() {
            return Ok(());
        }
        let backup: EnvBackup =
            serde_json::from_value(crate::config::read_json_file(&backup_path)?)
                .map_err(|e| AppError::json(&backup_path, e))?;
        // Keep a value someone else set meanwhile.
        let changed =
            if current.as_deref() == Some(VALUE) && backup.previous.as_deref() != Some(VALUE) {
                match backup.previous {
                    Some(previous) => env.set_value(NAME, &previous),
                    None => env.delete_value(NAME),
                }
                .map_err(|e| AppError::Message(format!("Could not restore {NAME}: {e}")))?;
                true
            } else {
                false
            };
        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
        changed
    };
    if changed {
        announce_environment_change();
    }
    Ok(())
}

#[cfg(not(windows))]
fn configure_cursor_env(_enabled: bool) -> Result<(), AppError> {
    Ok(())
}

/// Tells running programs that read it (Explorer, so terminals started from
/// it) that the user environment changed.
#[cfg(windows)]
fn announce_environment_change() {
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    let area: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr(),
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        );
    }
}

/// Release departed configuration directories before applying new ones.
pub fn reconfigure(previous: &AppSettings, next: &AppSettings) -> Result<(), AppError> {
    if previous.manual_sign_in
        && (previous.claude_config_dir != next.claude_config_dir
            || previous.claude_mirror_config_dir != next.claude_mirror_config_dir
            || previous.codex_config_dir != next.codex_config_dir
            || previous.codex_mirror_config_dir != next.codex_mirror_config_dir)
    {
        let mut off = previous.clone();
        off.manual_sign_in = false;
        apply(&off)?;
    }
    apply(next)
}

fn configure_claude(dir: &Path, enabled: bool, browser: &str) -> Result<(), AppError> {
    let settings_path = dir.join("settings.json");
    let backup_path = dir.join("switchy").join("manual_sign_in.json");
    if !enabled && !backup_path.exists() {
        return Ok(());
    }
    let had_settings = settings_path.exists();
    let mut settings: Value = if had_settings {
        crate::config::read_json_file(&settings_path)?
    } else {
        json!({})
    };
    if !settings.is_object() || settings.get("env").is_some_and(|env| !env.is_object()) {
        return Err(AppError::Config(format!(
            "Claude settings must contain an object with an optional env object: {}",
            settings_path.display()
        )));
    }
    let original = settings.clone();
    if enabled {
        if !backup_path.exists() {
            let current = settings.get("env").and_then(|env| env.get("BROWSER"));
            crate::config::write_json_file(
                &backup_path,
                &BrowserBackup {
                    had_browser: current.is_some(),
                    previous_browser: current.cloned().unwrap_or(Value::Null),
                    had_env: settings.get("env").is_some(),
                    had_settings,
                    installed_browser: browser.into(),
                },
            )?;
        } else {
            // The helper can move when the application's data directory moves.
            let mut backup = read_backup(&backup_path)?;
            if backup.installed_browser != browser {
                backup.installed_browser = browser.into();
                crate::config::write_json_file(&backup_path, &backup)?;
            }
        }
        if settings.get("env").is_none() {
            settings["env"] = json!({});
        }
        settings["env"]["BROWSER"] = Value::String(browser.into());
        if settings != original {
            crate::config::write_json_file(&settings_path, &settings)?;
        }
    } else {
        let backup = read_backup(&backup_path)?;
        // Keep a browser override another tool or the user changed meanwhile.
        if settings.get("env").and_then(|env| env.get("BROWSER"))
            == Some(&Value::String(backup.installed_browser))
        {
            let env = settings["env"].as_object_mut().unwrap();
            if backup.had_browser {
                env.insert("BROWSER".into(), backup.previous_browser);
            } else {
                env.remove("BROWSER");
            }
            if !backup.had_env && env.is_empty() {
                settings.as_object_mut().unwrap().remove("env");
            }
        }
        if !backup.had_settings && settings.as_object().unwrap().is_empty() {
            if had_settings {
                fs::remove_file(&settings_path).map_err(|e| AppError::io(&settings_path, e))?;
            }
        } else if settings != original {
            crate::config::write_json_file(&settings_path, &settings)?;
        }
        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_restores_the_browser_and_preserves_later_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        crate::config::write_json_file(
            &path,
            &json!({"env":{"BROWSER":"my-browser", "X":"keep"}, "hooks":{"old":true}}),
        )
        .unwrap();
        configure_claude(root.path(), true, "no-browser").unwrap();
        configure_claude(root.path(), true, "no-browser").unwrap();
        let mut current: Value = crate::config::read_json_file(&path).unwrap();
        current["hooks"]["new"] = json!(true);
        crate::config::write_json_file(&path, &current).unwrap();
        configure_claude(root.path(), false, "").unwrap();
        let restored: Value = crate::config::read_json_file(&path).unwrap();
        assert_eq!(
            restored,
            json!({"env":{"BROWSER":"my-browser", "X":"keep"}, "hooks":{"old":true, "new":true}})
        );
        assert!(!root.path().join("switchy/manual_sign_in.json").exists());
    }

    #[test]
    fn disabling_keeps_an_external_browser_change() {
        let root = tempfile::tempdir().unwrap();
        configure_claude(root.path(), true, "no-browser").unwrap();
        let path = root.path().join("settings.json");
        crate::config::write_json_file(&path, &json!({"env":{"BROWSER":"new-browser"}})).unwrap();
        configure_claude(root.path(), false, "").unwrap();
        let restored: Value = crate::config::read_json_file(&path).unwrap();
        assert_eq!(restored["env"]["BROWSER"], "new-browser");
    }

    #[test]
    fn a_generated_settings_file_is_removed_when_turned_off() {
        let root = tempfile::tempdir().unwrap();
        configure_claude(root.path(), false, "").unwrap();
        assert!(!root.path().join("settings.json").exists());
        configure_claude(root.path(), true, "no-browser").unwrap();
        configure_claude(root.path(), false, "").unwrap();
        assert!(!root.path().join("settings.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn embedded_browser_launcher_exits_without_opening_the_url() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("no_browser.exe");
        fs::write(
            &path,
            include_bytes!(concat!(env!("OUT_DIR"), "/no_browser.exe")),
        )
        .unwrap();
        assert!(std::process::Command::new(path)
            .arg("https://example.invalid")
            .status()
            .unwrap()
            .success());
    }
}
