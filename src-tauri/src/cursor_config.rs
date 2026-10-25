//! Cursor CLI (`cursor-agent`) login files.
//!
//! The CLI keeps its login in `auth.json` (`%APPDATA%\Cursor\auth.json` on
//! Windows, `$XDG_CONFIG_HOME/cursor/auth.json` on Linux) and the signed-in
//! account's identity under `authInfo` in `~/.cursor/cli-config.json`, next to
//! the user's own CLI settings. The Cursor IDE keeps its login in its own
//! database and does not read `auth.json`. A Switchy Cursor provider stores
//! `{ "auth": <auth.json | null>, "authInfo": <authInfo | null> }`.
//!
//! On macOS the CLI keeps the tokens in the Keychain, which is not handled.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::{delete_file, get_home_dir, read_json_file, write_json_file};
use crate::error::AppError;

fn sandboxed() -> bool {
    cfg!(test) || std::env::var_os(crate::paths::ENV_TEST_HOME).is_some()
}

/// The CLI's `auth.json`.
pub fn get_cursor_auth_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")
            .filter(|_| !sandboxed())
            .map(PathBuf::from)
            .unwrap_or_else(|| get_home_dir().join("AppData").join("Roaming"));
        appdata.join("Cursor").join("auth.json")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|_| !sandboxed())
            .map(PathBuf::from)
            .unwrap_or_else(|| get_home_dir().join(".config"));
        config.join("cursor").join("auth.json")
    }
}

/// `~/.cursor`, where the CLI keeps `cli-config.json`.
pub fn get_cursor_dir() -> PathBuf {
    get_home_dir().join(".cursor")
}

pub fn get_cursor_cli_config_path() -> PathBuf {
    get_cursor_dir().join("cli-config.json")
}

fn unsupported() -> Result<(), AppError> {
    if cfg!(target_os = "macos") && !sandboxed() {
        return Err(AppError::Message(
            "Cursor CLI keeps its login in the macOS Keychain, which Switchy does not switch yet"
                .to_string(),
        ));
    }
    Ok(())
}

/// The live login as a provider stores it.
pub fn read_cursor_live() -> Result<Value, AppError> {
    unsupported()?;
    read_live_at(&get_cursor_auth_path(), &get_cursor_cli_config_path())
}

fn read_live_at(auth_path: &Path, config_path: &Path) -> Result<Value, AppError> {
    let auth = if auth_path.exists() {
        read_json_file::<Value>(auth_path)?
    } else {
        Value::Null
    };
    let auth_info = if config_path.exists() {
        read_json_file::<Value>(config_path)?
            .get("authInfo")
            .cloned()
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({ "auth": auth, "authInfo": auth_info }))
}

/// Writes a provider's login: `auth.json`, and `authInfo` in
/// `cli-config.json` with every other key left as it is. A provider with no
/// login removes `auth.json`, so the CLI asks for `cursor-agent login`. When
/// the account changes, the CLI's cached privacy setting of the previous
/// account is dropped, as its own sign-in does.
pub fn write_cursor_live(settings: &Value) -> Result<(), AppError> {
    unsupported()?;
    write_live_at(
        &get_cursor_auth_path(),
        &get_cursor_cli_config_path(),
        settings,
    )
}

fn write_live_at(auth_path: &Path, config_path: &Path, settings: &Value) -> Result<(), AppError> {
    let auth = settings.get("auth").filter(|v| !v.is_null());
    let auth_info = settings.get("authInfo").cloned().unwrap_or(Value::Null);

    match auth {
        Some(auth) => {
            if let Some(parent) = auth_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            write_json_file(auth_path, auth)?;
        }
        None => {
            if auth_path.exists() {
                delete_file(auth_path)?;
            }
        }
    }

    if !config_path.exists() {
        // The CLI fetches its own identity when the file has none.
        return Ok(());
    }
    let mut config = read_json_file::<Value>(config_path)?;
    let Some(obj) = config.as_object_mut() else {
        return Ok(());
    };
    if obj.get("authInfo") == Some(&auth_info) {
        return Ok(());
    }
    let same_account = obj
        .get("authInfo")
        .and_then(|i| i.get("userId"))
        .is_some_and(|id| Some(id) == auth_info.get("userId"));
    if !same_account {
        obj.remove("privacyCache");
    }
    if auth_info.is_null() {
        obj.remove("authInfo");
    } else {
        obj.insert("authInfo".to_string(), auth_info);
    }
    write_json_file(config_path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_replaces_only_the_login_and_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("Cursor").join("auth.json");
        let config_path = dir.path().join("cli-config.json");
        std::fs::write(
            &config_path,
            r#"{"approvalMode":"unrestricted","privacyCache":{"privacyMode":3},"authInfo":{"email":"a@x.io","userId":1}}"#,
        )
        .unwrap();

        write_live_at(
            &auth_path,
            &config_path,
            &json!({
                "auth": { "accessToken": "B", "refreshToken": "B" },
                "authInfo": { "email": "b@x.io", "userId": 2 }
            }),
        )
        .unwrap();

        let config: Value = read_json_file(&config_path).unwrap();
        assert_eq!(config["approvalMode"], "unrestricted");
        assert_eq!(config["authInfo"]["email"], "b@x.io");
        assert!(config.get("privacyCache").is_none());
        let live = read_live_at(&auth_path, &config_path).unwrap();
        assert_eq!(live["auth"]["accessToken"], "B");

        write_live_at(
            &auth_path,
            &config_path,
            &json!({ "auth": null, "authInfo": null }),
        )
        .unwrap();
        assert!(!auth_path.exists());
        let config: Value = read_json_file(&config_path).unwrap();
        assert!(config.get("authInfo").is_none());
        assert_eq!(config["approvalMode"], "unrestricted");
    }
}
