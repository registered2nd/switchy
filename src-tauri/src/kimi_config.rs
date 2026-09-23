//! Kimi Code CLI configuration paths and live-file helpers.
//!
//! Kimi keeps everything under `~/.kimi-code/` (`$KIMI_CODE_HOME`):
//! `config.toml` (providers, models, `default_model`), `credentials/kimi-code.json`
//! (the OAuth login for the managed `managed:kimi-code` provider), `mcp.json`,
//! `AGENTS.md`, `skills/`, `sessions/`. A Switchy Kimi provider stores
//! `{ "config": "<config.toml text>", "credentials": <kimi-code.json | null> }`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{atomic_write, delete_file, get_home_dir, write_json_file, write_text_file};
use crate::error::AppError;

/// `~/.kimi-code`, or the directory override from settings.
pub fn get_kimi_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_kimi_override_dir() {
        return custom;
    }
    get_home_dir().join(".kimi-code")
}

pub fn get_kimi_config_path() -> PathBuf {
    get_kimi_dir().join("config.toml")
}

pub fn get_kimi_credentials_path() -> PathBuf {
    get_kimi_dir().join("credentials").join("kimi-code.json")
}

/// Reads `config.toml`; an absent file is an empty config.
pub fn read_kimi_config_text() -> Result<String, AppError> {
    let path = get_kimi_config_path();
    if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))
    } else {
        Ok(String::new())
    }
}

pub fn validate_config_toml(text: &str) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    toml::from_str::<toml::Table>(text)
        .map(|_| ())
        .map_err(|e| AppError::toml(Path::new("config.toml"), e))
}

pub fn read_and_validate_kimi_config_text() -> Result<String, AppError> {
    let s = read_kimi_config_text()?;
    validate_config_toml(&s)?;
    Ok(s)
}

/// Reads the managed-provider login; `Ok(None)` when there is no login.
pub fn read_kimi_credentials() -> Result<Option<Value>, AppError> {
    let path = get_kimi_credentials_path();
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| AppError::io(&path, e))?;
    serde_json::from_slice::<Value>(&bytes)
        .map(Some)
        .map_err(|e| AppError::json(&path, e))
}

/// Writes `config.toml` and `credentials/kimi-code.json` together, rolling the
/// credentials back if the config write fails. `credentials == None` (or JSON
/// null) means "no login": the credentials file is removed, the way switching
/// to a fresh Official Codex provider blanks `auth.json`.
pub fn write_kimi_live_atomic(
    credentials: Option<&Value>,
    config_text: &str,
) -> Result<(), AppError> {
    let cred_path = get_kimi_credentials_path();
    let config_path = get_kimi_config_path();

    validate_config_toml(config_text)?;
    if let Some(parent) = cred_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let old_credentials = if cred_path.exists() {
        Some(std::fs::read(&cred_path).map_err(|e| AppError::io(&cred_path, e))?)
    } else {
        None
    };

    match credentials.filter(|v| !v.is_null()) {
        Some(value) => write_json_file(&cred_path, value)?,
        None => {
            if cred_path.exists() {
                delete_file(&cred_path)?;
            }
        }
    }

    if let Err(e) = write_text_file(&config_path, config_text) {
        match old_credentials {
            Some(bytes) => {
                let _ = atomic_write(&cred_path, &bytes);
            }
            None => {
                let _ = delete_file(&cred_path);
            }
        }
        return Err(e);
    }
    Ok(())
}

/// Live `{ config, credentials }` as a Switchy provider stores it.
pub fn read_kimi_live() -> Result<Value, AppError> {
    let config = read_and_validate_kimi_config_text()?;
    let credentials = read_kimi_credentials()?.unwrap_or(Value::Null);
    Ok(serde_json::json!({ "config": config, "credentials": credentials }))
}

/// `default_model = "<providerId>/<model>"` → the provider id it names.
pub fn default_provider_id(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml_edit::DocumentMut>().ok()?;
    let alias = doc.get("default_model")?.as_str()?;
    alias.split_once('/').map(|(p, _)| p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_id_reads_alias_prefix() {
        let cfg = "default_model = \"kimi-code/kimi-for-coding\"\n";
        assert_eq!(default_provider_id(cfg).as_deref(), Some("kimi-code"));
        assert_eq!(default_provider_id("x = 1\n"), None);
        assert_eq!(default_provider_id("default_model = \"noslash\"\n"), None);
    }
}
