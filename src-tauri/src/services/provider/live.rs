//! Live configuration operations
//!
//! Handles reading and writing live configuration files for Claude, Codex, and Gemini.

use std::collections::HashMap;

use serde_json::{json, Value};
use toml_edit::{DocumentMut, Item, TableLike};

use crate::app_config::AppType;
use crate::codex_config::{get_codex_auth_path, get_codex_config_dir, get_codex_config_path};
use crate::config::{delete_file, get_claude_settings_path, read_json_file, write_json_file};
use crate::database::Database;
use crate::error::AppError;
use crate::provider::Provider;
use crate::store::AppState;

use super::gemini_auth::{
    detect_gemini_auth_type, ensure_google_oauth_security_flag, GeminiAuthType,
};
use super::normalize_claude_models_in_value;

pub(crate) fn sanitize_claude_settings_for_live(settings: &Value) -> Value {
    let mut v = settings.clone();
    if let Some(obj) = v.as_object_mut() {
        // Internal-only fields - never write to Claude Code settings.json
        obj.remove("api_format");
        obj.remove("apiFormat");
        obj.remove("openrouter_compat_mode");
        obj.remove("openrouterCompatMode");
    }
    v
}

// A provider owns only what chooses its endpoint, credentials and model. A
// switch writes that and leaves the rest of each settings file to the user
// and the other tools that write it (Orca's hooks, Claude Code's `/config`,
// Codex's own UI state): what a card stores beyond that is an old copy of
// the file, taken when the card was last switched away from, and writing it
// back would undo everything changed since.

/// Whether `key` is a Claude `env` key that chooses the endpoint, credentials
/// or models, as opposed to the user's own variables.
pub(crate) fn is_claude_connection_env_key(key: &str) -> bool {
    key.starts_with("ANTHROPIC_")
        || matches!(
            key,
            "CLAUDE_CODE_USE_BEDROCK"
                | "CLAUDE_CODE_USE_VERTEX"
                | "API_TIMEOUT_MS"
                | "GOOGLE_APPLICATION_CREDENTIALS"
                | "AWS_REGION"
                | "AWS_PROFILE"
                | "AWS_ACCESS_KEY_ID"
                | "AWS_SECRET_ACCESS_KEY"
                | "AWS_SESSION_TOKEN"
                | "ENABLE_TOOL_SEARCH"
        )
}

/// Top-level Claude settings a provider owns besides its `env` keys.
const CLAUDE_PROVIDER_OWNED_KEYS: [&str; 1] = ["apiKeyHelper"];

/// The part of Claude `settings` a provider owns.
pub(crate) fn claude_provider_owned(settings: &Value) -> Value {
    merge_claude_connection_into_target(&json!({}), settings)
}

/// `target` with the keys a provider owns replaced by `source`'s (removed
/// where `source` lacks them) and everything else kept.
pub(crate) fn merge_claude_connection_into_target(target: &Value, source: &Value) -> Value {
    let mut merged = target.clone();
    let Some(target_obj) = merged.as_object_mut() else {
        return claude_provider_owned(source);
    };
    let env = target_obj
        .entry("env".to_string())
        .or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env = env.as_object_mut().expect("env is an object");
    env.retain(|key, _| !is_claude_connection_env_key(key));
    if let Some(source_env) = source.get("env").and_then(Value::as_object) {
        for (key, value) in source_env {
            if is_claude_connection_env_key(key) {
                env.insert(key.clone(), value.clone());
            }
        }
    }
    if env.is_empty() {
        target_obj.remove("env");
    }
    for key in CLAUDE_PROVIDER_OWNED_KEYS {
        match source.get(key) {
            Some(value) => {
                target_obj.insert(key.to_string(), value.clone());
            }
            None => {
                target_obj.remove(key);
            }
        }
    }
    merged
}

/// `config.toml` keys an API provider owns: its routing, its model, and the
/// storage flag some relays require. An Official account owns none of the
/// file; its login is in `auth.json`.
const CODEX_RELAY_OWNED_KEYS: [&str; 4] = [
    "model_provider",
    "model_providers",
    "model",
    "disable_response_storage",
];

/// A Codex `config.toml` (`current`) after a switch to a provider whose stored
/// config is `config_text`. Leaving an API provider removes the keys it owns;
/// switching to one writes them; everything else in `current` stays. An
/// empty `current` gets the provider's config as it is.
pub(crate) fn codex_config_after_switch(
    current: &str,
    config_text: &str,
    official: bool,
) -> Result<String, AppError> {
    if current.trim().is_empty() {
        return Ok(config_text.to_string());
    }
    let mut doc = current
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;
    if doc.contains_key("model_provider") {
        for key in CODEX_RELAY_OWNED_KEYS {
            doc.remove(key);
        }
    }
    doc.remove("model_providers");
    if !official {
        let source = config_text
            .parse::<DocumentMut>()
            .map_err(|e| AppError::Message(format!("Invalid Codex provider config: {e}")))?;
        for key in CODEX_RELAY_OWNED_KEYS {
            if let Some(item) = source.get(key) {
                doc.insert(key, item.clone());
            }
        }
    }
    Ok(doc.to_string())
}

/// `config.toml` keys a Kimi provider owns: its default model and the
/// provider and model tables it names.
const KIMI_PROVIDER_OWNED_KEYS: [&str; 3] = ["default_model", "providers", "models"];

/// A Kimi `config.toml` (`current`) after a switch to a provider whose config
/// is `config_text`: the keys the provider owns are replaced, the rest stays.
pub(crate) fn kimi_config_after_switch(
    current: &str,
    config_text: &str,
) -> Result<String, AppError> {
    if current.trim().is_empty() {
        return Ok(config_text.to_string());
    }
    let mut doc = current
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Kimi config.toml: {e}")))?;
    let source = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Kimi provider config: {e}")))?;
    for key in KIMI_PROVIDER_OWNED_KEYS {
        doc.remove(key);
        if let Some(item) = source.get(key) {
            doc.insert(key, item.clone());
        }
    }
    Ok(doc.to_string())
}

/// Writes the Codex login and the keys the provider owns into a second
/// `~/.codex` (typically WSL). `auth.json` is copied whole; `config.toml`
/// keeps everything else that machine holds for itself.
fn write_codex_mirror(
    mirror_dir: &std::path::Path,
    auth: &Value,
    config_text: &str,
    official: bool,
) -> Result<(), AppError> {
    std::fs::create_dir_all(mirror_dir).map_err(|e| AppError::io(mirror_dir, e))?;
    write_json_file(&mirror_dir.join("auth.json"), auth)?;

    let config_path = mirror_dir.join("config.toml");
    let current = std::fs::read_to_string(&config_path).unwrap_or_default();
    if !current.trim().is_empty() && current.parse::<DocumentMut>().is_err() {
        log::warn!(
            "Codex mirror config '{}' is not valid TOML; leaving it untouched",
            config_path.display()
        );
        return Ok(());
    }
    let text = codex_config_after_switch(&current, config_text, official)?;
    crate::config::write_text_file(&config_path, &text)
}

/// Applies a change of the common config snippet (`old` to `new`) to the
/// app's live file: what the old snippet set is removed and the new one is
/// merged in; everything else in the file stays. Saving the common config is
/// how it reaches the tool, since a switch no longer writes it.
pub(crate) fn apply_common_config_change(
    app_type: &AppType,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<(), AppError> {
    match app_type {
        AppType::Claude => {
            let path = get_claude_settings_path();
            if !path.exists() {
                return Ok(());
            }
            let live: Value = read_json_file(&path)?;
            let without_old =
                remove_common_config_from_settings(app_type, &live, old.unwrap_or(""))?;
            let next = apply_common_config_to_settings(app_type, &without_old, new.unwrap_or(""))?;
            if next != live {
                write_json_file(&path, &next)?;
            }
        }
        AppType::Codex | AppType::Kimi => {
            let path = if matches!(app_type, AppType::Codex) {
                get_codex_config_path()
            } else {
                crate::kimi_config::get_kimi_config_path()
            };
            if !path.exists() {
                return Ok(());
            }
            let text = std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
            let live = json!({ "config": text });
            let without_old =
                remove_common_config_from_settings(app_type, &live, old.unwrap_or(""))?;
            let next = apply_common_config_to_settings(app_type, &without_old, new.unwrap_or(""))?;
            if let Some(next_text) = next.get("config").and_then(Value::as_str) {
                if next_text != text {
                    crate::config::write_text_file(&path, next_text)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn provider_exists_in_live_config(
    app_type: &AppType,
    provider_id: &str,
) -> Result<bool, AppError> {
    match app_type {
        AppType::OpenCode => crate::opencode_config::get_providers()
            .map(|providers| providers.contains_key(provider_id)),
        AppType::OpenClaw => crate::openclaw_config::get_providers()
            .map(|providers| providers.contains_key(provider_id)),
        _ => Ok(false),
    }
}

fn json_is_subset(target: &Value, source: &Value) -> bool {
    match source {
        Value::Object(source_map) => {
            let Some(target_map) = target.as_object() else {
                return false;
            };
            source_map.iter().all(|(key, source_value)| {
                target_map
                    .get(key)
                    .is_some_and(|target_value| json_is_subset(target_value, source_value))
            })
        }
        Value::Array(source_arr) => {
            let Some(target_arr) = target.as_array() else {
                return false;
            };
            json_array_contains_subset(target_arr, source_arr)
        }
        _ => target == source,
    }
}

fn json_array_contains_subset(target_arr: &[Value], source_arr: &[Value]) -> bool {
    let mut matched = vec![false; target_arr.len()];

    source_arr.iter().all(|source_item| {
        if let Some((index, _)) = target_arr.iter().enumerate().find(|(index, target_item)| {
            !matched[*index] && json_is_subset(target_item, source_item)
        }) {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn json_remove_array_items(target_arr: &mut Vec<Value>, source_arr: &[Value]) {
    for source_item in source_arr {
        if let Some(index) = target_arr
            .iter()
            .position(|target_item| json_is_subset(target_item, source_item))
        {
            target_arr.remove(index);
        }
    }
}

fn json_deep_merge(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                match target_map.get_mut(key) {
                    Some(target_value) => json_deep_merge(target_value, source_value),
                    None => {
                        target_map.insert(key.clone(), source_value.clone());
                    }
                }
            }
        }
        (target_value, source_value) => {
            *target_value = source_value.clone();
        }
    }
}

fn json_deep_remove(target: &mut Value, source: &Value) {
    let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };

    for (key, source_value) in source_map {
        let mut remove_key = false;

        if let Some(target_value) = target_map.get_mut(key) {
            if source_value.is_object() && target_value.is_object() {
                json_deep_remove(target_value, source_value);
                remove_key = target_value.as_object().is_some_and(|obj| obj.is_empty());
            } else if let (Some(target_arr), Some(source_arr)) =
                (target_value.as_array_mut(), source_value.as_array())
            {
                json_remove_array_items(target_arr, source_arr);
                remove_key = target_arr.is_empty();
            } else if json_is_subset(target_value, source_value) {
                remove_key = true;
            }
        }

        if remove_key {
            target_map.remove(key);
        }
    }
}

fn toml_value_is_subset(target: &toml_edit::Value, source: &toml_edit::Value) -> bool {
    match (target, source) {
        (toml_edit::Value::String(target), toml_edit::Value::String(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Integer(target), toml_edit::Value::Integer(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Float(target), toml_edit::Value::Float(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Boolean(target), toml_edit::Value::Boolean(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Datetime(target), toml_edit::Value::Datetime(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Array(target), toml_edit::Value::Array(source)) => {
            toml_array_contains_subset(target, source)
        }
        (toml_edit::Value::InlineTable(target), toml_edit::Value::InlineTable(source)) => {
            source.iter().all(|(key, source_item)| {
                target
                    .get(key)
                    .is_some_and(|target_item| toml_value_is_subset(target_item, source_item))
            })
        }
        _ => false,
    }
}

fn toml_array_contains_subset(target: &toml_edit::Array, source: &toml_edit::Array) -> bool {
    let mut matched = vec![false; target.len()];
    let target_items: Vec<&toml_edit::Value> = target.iter().collect();

    source.iter().all(|source_item| {
        if let Some((index, _)) = target_items
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && toml_value_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn toml_remove_array_items(target: &mut toml_edit::Array, source: &toml_edit::Array) {
    for source_item in source.iter() {
        let index = {
            let target_items: Vec<&toml_edit::Value> = target.iter().collect();
            target_items
                .iter()
                .enumerate()
                .find(|(_, target_item)| toml_value_is_subset(target_item, source_item))
                .map(|(index, _)| index)
        };

        if let Some(index) = index {
            target.remove(index);
        }
    }
}

fn toml_item_is_subset(target: &Item, source: &Item) -> bool {
    if let Some(source_table) = source.as_table_like() {
        let Some(target_table) = target.as_table_like() else {
            return false;
        };
        return source_table.iter().all(|(key, source_item)| {
            target_table
                .get(key)
                .is_some_and(|target_item| toml_item_is_subset(target_item, source_item))
        });
    }

    match (target.as_value(), source.as_value()) {
        (Some(target_value), Some(source_value)) => {
            toml_value_is_subset(target_value, source_value)
        }
        _ => false,
    }
}

fn merge_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            merge_toml_table_like(target_table, source_table);
            return;
        }
    }

    *target = source.clone();
}

fn merge_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    for (key, source_item) in source.iter() {
        match target.get_mut(key) {
            Some(target_item) => merge_toml_item(target_item, source_item),
            None => {
                target.insert(key, source_item.clone());
            }
        }
    }
}

fn remove_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            remove_toml_table_like(target_table, source_table);
            if target_table.is_empty() {
                *target = Item::None;
            }
            return;
        }
    }

    if let Some(source_value) = source.as_value() {
        let mut remove_item = false;

        if let Some(target_value) = target.as_value_mut() {
            match (target_value, source_value) {
                (toml_edit::Value::Array(target_arr), toml_edit::Value::Array(source_arr)) => {
                    toml_remove_array_items(target_arr, source_arr);
                    remove_item = target_arr.is_empty();
                }
                (target_value, source_value)
                    if toml_value_is_subset(target_value, source_value) =>
                {
                    remove_item = true;
                }
                _ => {}
            }
        }

        if remove_item {
            *target = Item::None;
        }
    }
}

fn remove_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    let keys: Vec<String> = source.iter().map(|(key, _)| key.to_string()).collect();

    for key in keys {
        let mut remove_key = false;
        if let (Some(target_item), Some(source_item)) = (target.get_mut(&key), source.get(&key)) {
            remove_toml_item(target_item, source_item);
            remove_key = target_item.is_none()
                || target_item
                    .as_table_like()
                    .is_some_and(|table_like| table_like.is_empty());
        }

        if remove_key {
            target.remove(&key);
        }
    }
}

fn settings_contain_common_config(app_type: &AppType, settings: &Value, snippet: &str) -> bool {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return false;
    }

    match app_type {
        AppType::Claude => match serde_json::from_str::<Value>(trimmed) {
            Ok(source) if source.is_object() => json_is_subset(settings, &source),
            _ => false,
        },
        AppType::Codex | AppType::Kimi => {
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            if config_toml.trim().is_empty() {
                return false;
            }

            let target_doc = match config_toml.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };
            let source_doc = match trimmed.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };

            toml_item_is_subset(target_doc.as_item(), source_doc.as_item())
        }
        AppType::Gemini => match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(source_map)) => {
                let Some(target_map) = settings.get("env").and_then(Value::as_object) else {
                    return false;
                };
                source_map.iter().all(|(key, source_value)| {
                    target_map
                        .get(key)
                        .is_some_and(|target_value| json_is_subset(target_value, source_value))
                })
            }
            _ => false,
        },
        AppType::OpenCode | AppType::OpenClaw => false,
    }
}

pub(crate) fn provider_uses_common_config(
    app_type: &AppType,
    provider: &Provider,
    snippet: Option<&str>,
) -> bool {
    match provider
        .meta
        .as_ref()
        .and_then(|meta| meta.common_config_enabled)
    {
        Some(explicit) => explicit && snippet.is_some_and(|value| !value.trim().is_empty()),
        None => snippet.is_some_and(|value| {
            settings_contain_common_config(app_type, &provider.settings_config, value)
        }),
    }
}

pub(crate) fn remove_common_config_from_settings(
    app_type: &AppType,
    settings: &Value,
    snippet: &str,
) -> Result<Value, AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(settings.clone());
    }

    match app_type {
        AppType::Claude => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Claude common config: {e}")))?;
            let mut result = settings.clone();
            json_deep_remove(&mut result, &source);
            Ok(result)
        }
        AppType::Codex | AppType::Kimi => {
            let mut result = settings.clone();
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let mut target_doc = if config_toml.trim().is_empty() {
                DocumentMut::new()
            } else {
                config_toml.parse::<DocumentMut>().map_err(|e| {
                    AppError::Message(format!(
                        "Invalid Codex config.toml while removing common config: {e}"
                    ))
                })?
            };
            let source_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
                AppError::Message(format!("Invalid Codex common config snippet: {e}"))
            })?;

            remove_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config".to_string(), Value::String(target_doc.to_string()));
            }
            Ok(result)
        }
        AppType::Gemini => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Gemini common config: {e}")))?;
            let mut result = settings.clone();
            if let Some(env) = result.get_mut("env") {
                json_deep_remove(env, &source);
            }
            Ok(result)
        }
        AppType::OpenCode | AppType::OpenClaw => Ok(settings.clone()),
    }
}

fn apply_common_config_to_settings(
    app_type: &AppType,
    settings: &Value,
    snippet: &str,
) -> Result<Value, AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(settings.clone());
    }

    match app_type {
        AppType::Claude => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Claude common config: {e}")))?;
            let mut result = settings.clone();
            json_deep_merge(&mut result, &source);
            Ok(result)
        }
        AppType::Codex | AppType::Kimi => {
            let mut result = settings.clone();
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let mut target_doc = if config_toml.trim().is_empty() {
                DocumentMut::new()
            } else {
                config_toml.parse::<DocumentMut>().map_err(|e| {
                    AppError::Message(format!(
                        "Invalid Codex config.toml while applying common config: {e}"
                    ))
                })?
            };
            let source_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
                AppError::Message(format!("Invalid Codex common config snippet: {e}"))
            })?;

            merge_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config".to_string(), Value::String(target_doc.to_string()));
            }
            Ok(result)
        }
        AppType::Gemini => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Gemini common config: {e}")))?;
            let mut result = settings.clone();
            if let Some(env) = result.get_mut("env") {
                json_deep_merge(env, &source);
            } else if let Some(obj) = result.as_object_mut() {
                obj.insert("env".to_string(), source);
            }
            Ok(result)
        }
        AppType::OpenCode | AppType::OpenClaw => Ok(settings.clone()),
    }
}

pub(crate) fn build_effective_settings_with_common_config(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
) -> Result<Value, AppError> {
    let snippet = db.get_config_snippet(app_type.as_str())?;
    let mut effective_settings = provider.settings_config.clone();

    if provider_uses_common_config(app_type, provider, snippet.as_deref()) {
        if let Some(snippet_text) = snippet.as_deref() {
            match apply_common_config_to_settings(app_type, &effective_settings, snippet_text) {
                Ok(settings) => effective_settings = settings,
                Err(err) => {
                    log::warn!(
                        "Failed to apply common config for {} provider '{}': {err}",
                        app_type.as_str(),
                        provider.id
                    );
                }
            }
        }
    }

    Ok(effective_settings)
}

pub(crate) fn write_live_with_common_config(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
) -> Result<(), AppError> {
    let mut effective_provider = provider.clone();
    effective_provider.settings_config =
        build_effective_settings_with_common_config(db, app_type, provider)?;

    write_live_snapshot(app_type, &effective_provider)
}

pub(crate) fn strip_common_config_from_live_settings(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
    live_settings: Value,
) -> Value {
    let snippet = match db.get_config_snippet(app_type.as_str()) {
        Ok(snippet) => snippet,
        Err(err) => {
            log::warn!(
                "Failed to load common config for {} while backfilling '{}': {err}",
                app_type.as_str(),
                provider.id
            );
            return live_settings;
        }
    };

    if !provider_uses_common_config(app_type, provider, snippet.as_deref()) {
        return live_settings;
    }

    let Some(snippet_text) = snippet.as_deref() else {
        return live_settings;
    };

    match remove_common_config_from_settings(app_type, &live_settings, snippet_text) {
        Ok(settings) => settings,
        Err(err) => {
            log::warn!(
                "Failed to strip common config for {} provider '{}': {err}",
                app_type.as_str(),
                provider.id
            );
            live_settings
        }
    }
}

pub(crate) fn normalize_provider_common_config_for_storage(
    db: &Database,
    app_type: &AppType,
    provider: &mut Provider,
) -> Result<(), AppError> {
    let uses_common_config = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.common_config_enabled)
        .unwrap_or(false);

    if !uses_common_config {
        return Ok(());
    }

    let Some(snippet) = db.get_config_snippet(app_type.as_str())? else {
        return Ok(());
    };

    if snippet.trim().is_empty() {
        return Ok(());
    }

    match remove_common_config_from_settings(app_type, &provider.settings_config, &snippet) {
        Ok(settings) => provider.settings_config = settings,
        Err(err) => {
            log::warn!(
                "Failed to normalize common config before saving {} provider '{}': {err}",
                app_type.as_str(),
                provider.id
            );
        }
    }

    Ok(())
}

/// Live configuration snapshot for backup/restore
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) enum LiveSnapshot {
    Claude {
        settings: Option<Value>,
    },
    Codex {
        auth: Option<Value>,
        config: Option<String>,
    },
    Gemini {
        env: Option<HashMap<String, String>>,
        config: Option<Value>,
    },
}

impl LiveSnapshot {
    #[allow(dead_code)]
    pub(crate) fn restore(&self) -> Result<(), AppError> {
        match self {
            LiveSnapshot::Claude { settings } => {
                let path = get_claude_settings_path();
                if let Some(value) = settings {
                    write_json_file(&path, value)?;
                } else if path.exists() {
                    delete_file(&path)?;
                }
            }
            LiveSnapshot::Codex { auth, config } => {
                let auth_path = get_codex_auth_path();
                let config_path = get_codex_config_path();
                if let Some(value) = auth {
                    write_json_file(&auth_path, value)?;
                } else if auth_path.exists() {
                    delete_file(&auth_path)?;
                }

                if let Some(text) = config {
                    crate::config::write_text_file(&config_path, text)?;
                } else if config_path.exists() {
                    delete_file(&config_path)?;
                }
            }
            LiveSnapshot::Gemini { env, .. } => {
                use crate::gemini_config::{
                    get_gemini_env_path, get_gemini_settings_path, write_gemini_env_atomic,
                };
                let path = get_gemini_env_path();
                if let Some(env_map) = env {
                    write_gemini_env_atomic(env_map)?;
                } else if path.exists() {
                    delete_file(&path)?;
                }

                let settings_path = get_gemini_settings_path();
                match self {
                    LiveSnapshot::Gemini {
                        config: Some(cfg), ..
                    } => {
                        write_json_file(&settings_path, cfg)?;
                    }
                    LiveSnapshot::Gemini { config: None, .. } if settings_path.exists() => {
                        delete_file(&settings_path)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// Write live configuration snapshot for a provider
pub(crate) fn write_live_snapshot(app_type: &AppType, provider: &Provider) -> Result<(), AppError> {
    match app_type {
        AppType::Claude => {
            let path = get_claude_settings_path();
            let settings = sanitize_claude_settings_for_live(&provider.settings_config);
            let existing_live = if path.exists() {
                read_json_file::<Value>(&path).ok()
            } else {
                None
            };
            match existing_live.as_ref() {
                Some(existing) => write_json_file(
                    &path,
                    &merge_claude_connection_into_target(existing, &settings),
                )?,
                None => write_json_file(&path, &settings)?,
            }

            if let Some(mirror_dir) = crate::settings::get_claude_mirror_override_dir() {
                let mirror_path = if mirror_dir.join("settings.json").exists() {
                    mirror_dir.join("settings.json")
                } else if mirror_dir.join("claude.json").exists() {
                    mirror_dir.join("claude.json")
                } else {
                    mirror_dir.join("settings.json")
                };

                if mirror_path != path {
                    let mirror_settings = if mirror_path.exists() {
                        match read_json_file::<Value>(&mirror_path) {
                            Ok(existing) => {
                                merge_claude_connection_into_target(&existing, &settings)
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to read Claude mirror config '{}': {err}. Skipping mirror sync to avoid overwriting machine-specific settings.",
                                    mirror_path.display()
                                );
                                return Ok(());
                            }
                        }
                    } else {
                        claude_provider_owned(&settings)
                    };

                    write_json_file(&mirror_path, &mirror_settings)?;
                }
            }
        }
        AppType::Codex => {
            let obj = provider.settings_config.as_object().ok_or_else(|| {
                AppError::Config("Codex provider config must be a JSON object".to_string())
            })?;
            let auth = obj.get("auth").ok_or_else(|| {
                AppError::Config("Codex provider config is missing the 'auth' field".to_string())
            })?;
            let config_str = obj.get("config").and_then(|v| v.as_str()).ok_or_else(|| {
                AppError::Config(
                    "Codex provider config is missing the 'config' field or it is not a string"
                        .to_string(),
                )
            })?;

            let official = provider.category.as_deref() == Some("official");
            let auth_path = get_codex_auth_path();
            write_json_file(&auth_path, auth)?;
            let config_path = get_codex_config_path();
            let current = std::fs::read_to_string(&config_path).unwrap_or_default();
            match codex_config_after_switch(&current, config_str, official) {
                Ok(text) => {
                    if text != current {
                        crate::config::write_text_file(&config_path, &text)?;
                    }
                }
                Err(err) => log::warn!(
                    "Codex config '{}' is not valid TOML; leaving it untouched: {err}",
                    config_path.display()
                ),
            }

            // Second install (typically WSL): same login, the keys the
            // provider owns written over whatever that machine keeps.
            if let Some(mirror_dir) = crate::settings::get_codex_mirror_override_dir() {
                if mirror_dir != get_codex_config_dir() {
                    if let Err(err) = write_codex_mirror(&mirror_dir, auth, config_str, official) {
                        log::warn!(
                            "Failed to write Codex mirror '{}': {err}. Live config was written; mirror left as-is.",
                            mirror_dir.display()
                        );
                    }
                }
            }
        }
        AppType::Gemini => {
            // Delegate to write_gemini_live which handles env file writing correctly
            write_gemini_live(provider)?;
        }
        AppType::Kimi => {
            let obj = provider.settings_config.as_object().ok_or_else(|| {
                AppError::Config("Kimi provider config must be a JSON object".to_string())
            })?;
            let config_str = obj.get("config").and_then(Value::as_str).ok_or_else(|| {
                AppError::Config(
                    "Kimi provider config is missing the 'config' field or it is not a string"
                        .to_string(),
                )
            })?;
            let current = std::fs::read_to_string(crate::kimi_config::get_kimi_config_path())
                .unwrap_or_default();
            let config_text = kimi_config_after_switch(&current, config_str)?;
            crate::kimi_config::write_kimi_live_atomic(obj.get("credentials"), &config_text)?;
        }
        AppType::OpenCode => {
            // OpenCode uses additive mode - write provider to config
            use crate::opencode_config;
            use crate::provider::OpenCodeProviderConfig;

            // Defensive check: if settings_config is a full config structure, extract provider fragment
            let config_to_write = if let Some(obj) = provider.settings_config.as_object() {
                // Detect full config structure (has $schema or top-level provider field)
                if obj.contains_key("$schema") || obj.contains_key("provider") {
                    log::warn!(
                        "OpenCode provider '{}' has full config structure in settings_config, attempting to extract fragment",
                        provider.id
                    );
                    // Try to extract from provider.{id}
                    obj.get("provider")
                        .and_then(|p| p.get(&provider.id))
                        .cloned()
                        .unwrap_or_else(|| provider.settings_config.clone())
                } else {
                    provider.settings_config.clone()
                }
            } else {
                provider.settings_config.clone()
            };

            // Convert settings_config to OpenCodeProviderConfig
            let opencode_config_result =
                serde_json::from_value::<OpenCodeProviderConfig>(config_to_write.clone());

            match opencode_config_result {
                Ok(config) => {
                    opencode_config::set_typed_provider(&provider.id, &config)?;
                    log::info!("OpenCode provider '{}' written to live config", provider.id);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to parse OpenCode provider config for '{}': {}",
                        provider.id,
                        e
                    );
                    // Only write if config looks like a valid provider fragment
                    if config_to_write.get("npm").is_some()
                        || config_to_write.get("options").is_some()
                    {
                        opencode_config::set_provider(&provider.id, config_to_write)?;
                        log::info!(
                            "OpenCode provider '{}' written as raw JSON to live config",
                            provider.id
                        );
                    } else {
                        return Err(AppError::Message(format!(
                            "OpenCode provider '{}' has invalid config structure for live config (must contain 'npm' or 'options')",
                            provider.id
                        )));
                    }
                }
            }
        }
        AppType::OpenClaw => {
            // OpenClaw uses additive mode - write provider to config
            use crate::openclaw_config;
            use crate::openclaw_config::OpenClawProviderConfig;

            // Convert settings_config to OpenClawProviderConfig
            let openclaw_config_result =
                serde_json::from_value::<OpenClawProviderConfig>(provider.settings_config.clone());

            match openclaw_config_result {
                Ok(config) => {
                    openclaw_config::set_typed_provider(&provider.id, &config)?;
                    log::info!("OpenClaw provider '{}' written to live config", provider.id);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to parse OpenClaw provider config for '{}': {}",
                        provider.id,
                        e
                    );
                    // Try to write as raw JSON if it looks valid
                    if provider.settings_config.get("baseUrl").is_some()
                        || provider.settings_config.get("api").is_some()
                        || provider.settings_config.get("models").is_some()
                    {
                        openclaw_config::set_provider(
                            &provider.id,
                            provider.settings_config.clone(),
                        )?;
                        log::info!(
                            "OpenClaw provider '{}' written as raw JSON to live config",
                            provider.id
                        );
                    } else {
                        return Err(AppError::Message(format!(
                            "OpenClaw provider '{}' has invalid config structure for live config (must contain 'baseUrl', 'api', or 'models')",
                            provider.id
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Sync all providers to live configuration (for additive mode apps)
///
/// Writes all providers from the database to the live configuration file.
/// Used for OpenCode and other additive mode applications.
fn sync_all_providers_to_live(state: &AppState, app_type: &AppType) -> Result<(), AppError> {
    let providers = state.db.get_all_providers(app_type.as_str())?;
    let mut synced_count = 0usize;

    for provider in providers.values() {
        if provider
            .meta
            .as_ref()
            .and_then(|meta| meta.live_config_managed)
            == Some(false)
        {
            continue;
        }

        if let Err(e) = write_live_with_common_config(state.db.as_ref(), app_type, provider) {
            log::warn!(
                "Failed to sync {:?} provider '{}' to live: {e}",
                app_type,
                provider.id
            );
            continue;
        }
        synced_count += 1;
    }

    log::info!("Synced {synced_count} {app_type:?} providers to live config");
    Ok(())
}

pub(crate) fn sync_current_provider_for_app_to_live(
    state: &AppState,
    app_type: &AppType,
) -> Result<(), AppError> {
    if app_type.is_additive_mode() {
        sync_all_providers_to_live(state, app_type)?;
    } else {
        let current_id = match crate::settings::get_effective_current_provider(&state.db, app_type)?
        {
            Some(id) => id,
            None => return Ok(()),
        };

        let providers = state.db.get_all_providers(app_type.as_str())?;
        if let Some(provider) = providers.get(&current_id) {
            write_live_with_common_config(state.db.as_ref(), app_type, provider)?;
        }
    }

    Ok(())
}

/// Sync current provider to live configuration
///
/// Use the effective current provider ID (checked to exist).
/// Reads local settings first; after validation, falls back to the database's is_current field.
/// This way an invalid ID left by a config import falls back to the database automatically.
///
/// For additive mode apps (OpenCode), all providers are synced instead of just the current one.
pub fn sync_current_to_live(state: &AppState) -> Result<(), AppError> {
    // Sync providers based on mode
    for app_type in AppType::all() {
        if app_type.is_additive_mode() {
            // Additive mode: sync ALL providers
            sync_all_providers_to_live(state, &app_type)?;
        } else {
            // Switch mode: sync only current provider
            let current_id =
                match crate::settings::get_effective_current_provider(&state.db, &app_type)? {
                    Some(id) => id,
                    None => continue,
                };

            let providers = state.db.get_all_providers(app_type.as_str())?;
            if let Some(provider) = providers.get(&current_id) {
                write_live_with_common_config(state.db.as_ref(), &app_type, provider)?;
            }
            // Note: get_effective_current_provider already validates existence,
            // so providers.get() should always succeed here
        }
    }

    Ok(())
}

/// Read current live settings for an app type
pub fn read_live_settings(app_type: AppType) -> Result<Value, AppError> {
    match app_type {
        AppType::Codex => {
            let auth_path = get_codex_auth_path();
            if !auth_path.exists() {
                return Err(AppError::localized(
                    "codex.auth.missing",
                    "Codex configuration missing: auth.json not found",
                ));
            }
            let auth: Value = read_json_file(&auth_path)?;
            let cfg_text = crate::codex_config::read_and_validate_codex_config_text()?;
            Ok(json!({ "auth": auth, "config": cfg_text }))
        }
        AppType::Claude => {
            let path = get_claude_settings_path();
            if !path.exists() {
                return Err(AppError::localized(
                    "claude.live.missing",
                    "Claude settings file is missing",
                ));
            }
            read_json_file(&path)
        }
        AppType::Kimi => {
            let config_path = crate::kimi_config::get_kimi_config_path();
            if !config_path.exists() {
                return Err(AppError::localized(
                    "kimi.config.missing",
                    "Kimi configuration missing: config.toml not found",
                ));
            }
            crate::kimi_config::read_kimi_live()
        }
        AppType::Gemini => {
            use crate::gemini_config::{
                env_to_json, get_gemini_env_path, get_gemini_settings_path, read_gemini_env,
            };

            // Read .env file (environment variables)
            let env_path = get_gemini_env_path();
            if !env_path.exists() {
                return Err(AppError::localized(
                    "gemini.env.missing",
                    "Gemini .env file not found",
                ));
            }

            let env_map = read_gemini_env()?;
            let env_json = env_to_json(&env_map);
            let env_obj = env_json.get("env").cloned().unwrap_or_else(|| json!({}));

            // Read settings.json file (MCP config etc.)
            let settings_path = get_gemini_settings_path();
            let config_obj = if settings_path.exists() {
                read_json_file(&settings_path)?
            } else {
                json!({})
            };

            // Return complete structure: { "env": {...}, "config": {...} }
            Ok(json!({
                "env": env_obj,
                "config": config_obj
            }))
        }
        AppType::OpenCode => {
            use crate::opencode_config::{get_opencode_config_path, read_opencode_config};

            let config_path = get_opencode_config_path();
            if !config_path.exists() {
                return Err(AppError::localized(
                    "opencode.config.missing",
                    "OpenCode configuration file not found",
                ));
            }

            let config = read_opencode_config()?;
            Ok(config)
        }
        AppType::OpenClaw => {
            use crate::openclaw_config::{get_openclaw_config_path, read_openclaw_config};

            let config_path = get_openclaw_config_path();
            if !config_path.exists() {
                return Err(AppError::localized(
                    "openclaw.config.missing",
                    "OpenClaw configuration file not found",
                ));
            }

            let config = read_openclaw_config()?;
            Ok(config)
        }
    }
}

/// Import default configuration from live files
///
/// Returns `Ok(true)` if a provider was actually imported,
/// `Ok(false)` if skipped (providers already exist for this app).
pub fn import_default_config(state: &AppState, app_type: AppType) -> Result<bool, AppError> {
    // Additive mode apps (OpenCode, OpenClaw) should use their dedicated
    // import_xxx_providers_from_live functions, not this generic default config import
    if app_type.is_additive_mode() {
        return Ok(false);
    }

    {
        let providers = state.db.get_all_providers(app_type.as_str())?;
        if !providers.is_empty() {
            return Ok(false); // Provider already exists; skip
        }
    }

    let settings_config = match app_type {
        AppType::Codex => {
            let auth_path = get_codex_auth_path();
            if !auth_path.exists() {
                return Err(AppError::localized(
                    "codex.live.missing",
                    "Codex configuration file is missing",
                ));
            }
            let auth: Value = read_json_file(&auth_path)?;
            let config_str = crate::codex_config::read_and_validate_codex_config_text()?;
            json!({ "auth": auth, "config": config_str })
        }
        AppType::Claude => {
            let settings_path = get_claude_settings_path();
            if !settings_path.exists() {
                return Err(AppError::localized(
                    "claude.live.missing",
                    "Claude settings file is missing",
                ));
            }
            let mut v = read_json_file::<Value>(&settings_path)?;
            let _ = normalize_claude_models_in_value(&mut v);
            v
        }
        AppType::Kimi => {
            if !crate::kimi_config::get_kimi_config_path().exists() {
                return Err(AppError::localized(
                    "kimi.live.missing",
                    "Kimi configuration file is missing",
                ));
            }
            crate::kimi_config::read_kimi_live()?
        }
        AppType::Gemini => {
            use crate::gemini_config::{
                env_to_json, get_gemini_env_path, get_gemini_settings_path, read_gemini_env,
            };

            // Read .env file (environment variables)
            let env_path = get_gemini_env_path();
            if !env_path.exists() {
                return Err(AppError::localized(
                    "gemini.live.missing",
                    "Gemini configuration file is missing",
                ));
            }

            let env_map = read_gemini_env()?;
            let env_json = env_to_json(&env_map);
            let env_obj = env_json.get("env").cloned().unwrap_or_else(|| json!({}));

            // Read settings.json file (MCP config etc.)
            let settings_path = get_gemini_settings_path();
            let config_obj = if settings_path.exists() {
                read_json_file(&settings_path)?
            } else {
                json!({})
            };

            // Return complete structure: { "env": {...}, "config": {...} }
            json!({
                "env": env_obj,
                "config": config_obj
            })
        }
        // OpenCode and OpenClaw use additive mode and are handled by early return above
        AppType::OpenCode | AppType::OpenClaw => {
            unreachable!("additive mode apps are handled by early return")
        }
    };

    let mut provider = Provider::with_id(
        "default".to_string(),
        "default".to_string(),
        settings_config,
        None,
    );
    provider.category = Some("custom".to_string());

    state.db.save_provider(app_type.as_str(), &provider)?;
    state
        .db
        .set_current_provider(app_type.as_str(), &provider.id)?;

    Ok(true) // Actually imported
}

/// Write Gemini live configuration with authentication handling
pub(crate) fn write_gemini_live(provider: &Provider) -> Result<(), AppError> {
    use crate::gemini_config::{
        get_gemini_settings_path, json_to_env, validate_gemini_settings_strict,
        write_gemini_env_atomic,
    };

    // One-time auth type detection to avoid repeated detection
    let auth_type = detect_gemini_auth_type(provider);

    let mut env_map = json_to_env(&provider.settings_config)?;

    // Prepare config to write to ~/.gemini/settings.json
    // Behavior:
    // - config is object: use it (merge with existing to preserve mcpServers etc.)
    // - config is null or absent: preserve existing file content
    let settings_path = get_gemini_settings_path();
    let mut config_to_write: Option<Value> = None;

    if let Some(config_value) = provider.settings_config.get("config") {
        if config_value.is_object() {
            // Merge with existing settings to preserve mcpServers and other fields
            let mut merged = if settings_path.exists() {
                read_json_file::<Value>(&settings_path).unwrap_or_else(|_| json!({}))
            } else {
                json!({})
            };

            // Merge provider config into existing settings
            if let (Some(merged_obj), Some(config_obj)) =
                (merged.as_object_mut(), config_value.as_object())
            {
                for (k, v) in config_obj {
                    merged_obj.insert(k.clone(), v.clone());
                }
            }
            config_to_write = Some(merged);
        } else if !config_value.is_null() {
            return Err(AppError::localized(
                "gemini.validation.invalid_config",
                "Gemini config invalid: config must be an object or null",
            ));
        }
        // config is null: don't modify existing settings.json (preserve mcpServers etc.)
    }

    // If no config specified or config is null, preserve existing file
    if config_to_write.is_none() && settings_path.exists() {
        config_to_write = Some(read_json_file(&settings_path)?);
    }

    match auth_type {
        GeminiAuthType::GoogleOfficial => {
            // Google official uses OAuth, clear env
            env_map.clear();
            write_gemini_env_atomic(&env_map)?;
        }
        GeminiAuthType::Packycode => {
            // PackyCode provider, uses API Key (strict validation on switch)
            validate_gemini_settings_strict(&provider.settings_config)?;
            write_gemini_env_atomic(&env_map)?;
        }
        GeminiAuthType::Generic => {
            // Generic provider, uses API Key (strict validation on switch)
            validate_gemini_settings_strict(&provider.settings_config)?;
            write_gemini_env_atomic(&env_map)?;
        }
    }

    if let Some(config_value) = config_to_write {
        write_json_file(&settings_path, &config_value)?;
    }

    // Set security.auth.selectedType based on auth type
    // - Google Official: OAuth mode
    // - All others: API Key mode
    match auth_type {
        GeminiAuthType::GoogleOfficial => ensure_google_oauth_security_flag(provider)?,
        GeminiAuthType::Packycode | GeminiAuthType::Generic => {
            crate::gemini_config::write_packycode_settings()?;
        }
    }

    Ok(())
}

/// Remove an OpenCode provider from the live configuration
///
/// This is specific to OpenCode's additive mode - removing a provider
/// from the opencode.json file.
pub(crate) fn remove_opencode_provider_from_live(provider_id: &str) -> Result<(), AppError> {
    use crate::opencode_config;

    // Check if OpenCode config directory exists
    if !opencode_config::get_opencode_dir().exists() {
        log::debug!("OpenCode config directory doesn't exist, skipping removal of '{provider_id}'");
        return Ok(());
    }

    opencode_config::remove_provider(provider_id)?;
    log::info!("OpenCode provider '{provider_id}' removed from live config");

    Ok(())
}

/// Import all providers from OpenCode live config to database
///
/// This imports existing providers from ~/.config/opencode/opencode.json
/// into the Switchy database. Each provider found will be added to the
/// database with is_current set to false.
pub fn import_opencode_providers_from_live(state: &AppState) -> Result<usize, AppError> {
    use crate::opencode_config;

    let providers = opencode_config::get_typed_providers()?;
    if providers.is_empty() {
        return Ok(0);
    }

    let mut imported = 0;
    let existing = state.db.get_all_providers("opencode")?;

    for (id, config) in providers {
        // Skip if already exists in database
        if existing.contains_key(&id) {
            log::debug!("OpenCode provider '{id}' already exists in database, skipping");
            continue;
        }

        // Convert to Value for settings_config
        let settings_config = match serde_json::to_value(&config) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to serialize OpenCode provider '{id}': {e}");
                continue;
            }
        };

        // Create provider
        let mut provider = Provider::with_id(
            id.clone(),
            config.name.clone().unwrap_or_else(|| id.clone()),
            settings_config,
            None,
        );
        provider.meta = Some(crate::provider::ProviderMeta {
            live_config_managed: Some(true),
            ..Default::default()
        });

        // Save to database
        if let Err(e) = state.db.save_provider("opencode", &provider) {
            log::warn!("Failed to import OpenCode provider '{id}': {e}");
            continue;
        }

        imported += 1;
        log::info!("Imported OpenCode provider '{id}' from live config");
    }

    Ok(imported)
}

/// Import all providers from OpenClaw live config to database
///
/// This imports existing providers from ~/.openclaw/openclaw.json
/// into the Switchy database. Each provider found will be added to the
/// database with is_current set to false.
pub fn import_openclaw_providers_from_live(state: &AppState) -> Result<usize, AppError> {
    use crate::openclaw_config;

    let providers = openclaw_config::get_typed_providers()?;
    if providers.is_empty() {
        return Ok(0);
    }

    let mut imported = 0;
    let existing = state.db.get_all_providers("openclaw")?;

    for (id, config) in providers {
        // Validate: skip entries with empty id or no models
        if id.trim().is_empty() {
            log::warn!("Skipping OpenClaw provider with empty id");
            continue;
        }
        if config.models.is_empty() {
            log::warn!("Skipping OpenClaw provider '{id}': no models defined");
            continue;
        }

        // Skip if already exists in database
        if existing.contains_key(&id) {
            log::debug!("OpenClaw provider '{id}' already exists in database, skipping");
            continue;
        }

        // Convert to Value for settings_config
        let settings_config = match serde_json::to_value(&config) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to serialize OpenClaw provider '{id}': {e}");
                continue;
            }
        };

        // Determine display name: use first model name if available, otherwise use id
        let display_name = config
            .models
            .first()
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| id.clone());

        // Create provider
        let mut provider = Provider::with_id(id.clone(), display_name, settings_config, None);
        provider.meta = Some(crate::provider::ProviderMeta {
            live_config_managed: Some(true),
            ..Default::default()
        });

        // Save to database
        if let Err(e) = state.db.save_provider("openclaw", &provider) {
            log::warn!("Failed to import OpenClaw provider '{id}': {e}");
            continue;
        }

        imported += 1;
        log::info!("Imported OpenClaw provider '{id}' from live config");
    }

    Ok(imported)
}

/// Remove an OpenClaw provider from live config
///
/// This removes a specific provider from ~/.openclaw/openclaw.json
/// without affecting other providers in the file.
pub fn remove_openclaw_provider_from_live(provider_id: &str) -> Result<(), AppError> {
    use crate::openclaw_config;

    // Check if OpenClaw config directory exists
    if !openclaw_config::get_openclaw_dir().exists() {
        log::debug!("OpenClaw config directory doesn't exist, skipping removal of '{provider_id}'");
        return Ok(());
    }

    openclaw_config::remove_provider(provider_id)?;
    log::info!("OpenClaw provider '{provider_id}' removed from live config");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_common_config_apply_and_remove_roundtrip_for_non_overlapping_fields() {
        let settings = json!({
            "env": {
                "ANTHROPIC_API_KEY": "sk-test"
            }
        });
        let snippet = r#"{
  "includeCoAuthoredBy": false,
  "env": {
    "CLAUDE_CODE_USE_BEDROCK": "1"
  }
}"#;

        let applied =
            apply_common_config_to_settings(&AppType::Claude, &settings, snippet).unwrap();
        assert_eq!(applied["includeCoAuthoredBy"], json!(false));
        assert_eq!(applied["env"]["CLAUDE_CODE_USE_BEDROCK"], json!("1"));

        let stripped =
            remove_common_config_from_settings(&AppType::Claude, &applied, snippet).unwrap();
        assert_eq!(stripped, settings);
    }

    #[test]
    fn codex_common_config_apply_and_remove_roundtrip_for_non_overlapping_fields() {
        let settings = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "config": "model_provider = \"openai\"\n[general]\nmodel = \"gpt-5\"\n"
        });
        let snippet = "[shared]\nreasoning = \"medium\"\n";

        let applied = apply_common_config_to_settings(&AppType::Codex, &settings, snippet).unwrap();
        let applied_config = applied["config"].as_str().unwrap_or_default();
        assert!(applied_config.contains("[shared]"));
        assert!(applied_config.contains("reasoning = \"medium\""));

        let stripped =
            remove_common_config_from_settings(&AppType::Codex, &applied, snippet).unwrap();
        assert_eq!(stripped, settings);
    }

    #[test]
    fn explicit_common_config_flag_overrides_legacy_subset_detection() {
        let mut provider = Provider::with_id(
            "claude-test".to_string(),
            "Claude Test".to_string(),
            json!({
                "includeCoAuthoredBy": false
            }),
            None,
        );
        provider.meta = Some(crate::provider::ProviderMeta {
            common_config_enabled: Some(false),
            ..Default::default()
        });

        assert!(
            !provider_uses_common_config(
                &AppType::Claude,
                &provider,
                Some(r#"{ "includeCoAuthoredBy": false }"#),
            ),
            "explicit false should win over legacy subset detection"
        );
    }

    #[test]
    fn claude_common_config_array_subset_detection_and_strip_preserve_extra_items() {
        let settings = json!({
            "allowedTools": ["tool1", "tool2"]
        });
        let snippet = r#"{
  "allowedTools": ["tool1"]
}"#;

        assert!(
            settings_contain_common_config(&AppType::Claude, &settings, snippet),
            "array subset should be detected for legacy providers"
        );

        let stripped =
            remove_common_config_from_settings(&AppType::Claude, &settings, snippet).unwrap();
        assert_eq!(
            stripped,
            json!({
                "allowedTools": ["tool2"]
            })
        );
    }

    #[test]
    fn switching_to_an_official_codex_account_leaves_config_toml_alone() {
        let live = "model = \"gpt-6-astra\"
approval_policy = \"never\"

[mcp_servers.mine]
command = \"x\"
";
        let card = "model = \"old\"

[mcp_servers.card]
command = \"y\"
";
        assert_eq!(codex_config_after_switch(live, card, true).unwrap(), live);
    }

    #[test]
    fn a_codex_relay_writes_its_routing_and_leaving_it_removes_them() {
        let live = "approval_policy = \"never\"
model = \"gpt-6-astra\"
";
        let relay = "model_provider = \"relay\"
model = \"glm\"
sandbox_mode = \"read-only\"

[model_providers.relay]
base_url = \"https://relay.example/v1\"
";
        let on_relay = codex_config_after_switch(live, relay, false).unwrap();
        let table: toml::Table = on_relay.parse().unwrap();
        assert_eq!(table["model_provider"].as_str(), Some("relay"));
        assert_eq!(table["model"].as_str(), Some("glm"));
        assert!(table.contains_key("model_providers"));
        assert_eq!(table["approval_policy"].as_str(), Some("never"));
        assert!(!table.contains_key("sandbox_mode"), "only the relay's own keys are written");

        let back: toml::Table = codex_config_after_switch(&on_relay, "", true)
            .unwrap()
            .parse()
            .unwrap();
        assert!(!back.contains_key("model_provider"));
        assert!(!back.contains_key("model_providers"));
        assert!(!back.contains_key("model"), "the relay's model goes with it");
        assert_eq!(back["approval_policy"].as_str(), Some("never"));
    }

    #[test]
    fn a_kimi_switch_replaces_only_the_provider_keys() {
        let live = "default_model = \"a/m\"
default_permission_mode = \"yolo\"

[providers.a]
base_url = \"https://a\"
";
        let card = "default_model = \"b/m\"
default_permission_mode = \"ask\"

[providers.b]
base_url = \"https://b\"
";
        let table: toml::Table = kimi_config_after_switch(live, card).unwrap().parse().unwrap();
        assert_eq!(table["default_model"].as_str(), Some("b/m"));
        assert!(table["providers"].get("b").is_some());
        assert!(table["providers"].get("a").is_none());
        assert_eq!(table["default_permission_mode"].as_str(), Some("yolo"));
    }

    #[test]
    fn a_claude_switch_writes_only_the_connection_keys() {
        let target = json!({
            "env": {
                "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": "95"
            },
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "python3 /home/agentcode/.claude/hooks/dashboard-hook.py"
                            }
                        ]
                    }
                ]
            },
            "statusLine": {
                "type": "command",
                "command": "python3 /home/agentcode/.claude/status.py"
            }
        });
        let source = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "https://www.right.codes/claude",
                "ENABLE_TOOL_SEARCH": "true",
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000"
            },
            "model": "opus",
            "permissions": {
                "defaultMode": "bypassPermissions"
            },
            "effortLevel": "medium",
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "node C:/broken/windows/hook.js"
                            }
                        ]
                    }
                ]
            }
        });

        let merged = merge_claude_connection_into_target(&target, &source);
        assert_eq!(
            merged["env"],
            json!({
                "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": "95",
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "https://www.right.codes/claude",
                "ENABLE_TOOL_SEARCH": "true"
            }),
            "the connection keys come from the provider; the machine's own variables stay"
        );
        assert!(
            merged.get("model").is_none(),
            "the model is the user's, not the provider's"
        );
        assert!(merged.get("permissions").is_none());
        assert_eq!(
            merged["hooks"]["Stop"][0]["hooks"][0]["command"],
            json!("python3 /home/agentcode/.claude/hooks/dashboard-hook.py")
        );
        assert_eq!(
            merged["statusLine"]["command"],
            json!("python3 /home/agentcode/.claude/status.py")
        );
    }

    #[test]
    fn a_new_claude_mirror_file_gets_only_the_connection_keys() {
        let source = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "https://www.right.codes/claude",
                "ENABLE_TOOL_SEARCH": "true",
                "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": "95"
            },
            "model": "opus",
            "permissions": {
                "defaultMode": "bypassPermissions"
            },
            "effortLevel": "medium",
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "node C:/broken/windows/hook.js"
                            }
                        ]
                    }
                ]
            }
        });

        let mirror = claude_provider_owned(&source);
        assert_eq!(
            mirror,
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "sk-test",
                    "ANTHROPIC_BASE_URL": "https://www.right.codes/claude",
                    "ENABLE_TOOL_SEARCH": "true"
                }
            })
        );
    }

    #[test]
    fn codex_common_config_array_subset_detection_and_strip_preserve_extra_items() {
        let settings = json!({
            "auth": {},
            "config": "allowed_tools = [\"tool1\", \"tool2\"]\n"
        });
        let snippet = "allowed_tools = [\"tool1\"]\n";

        assert!(
            settings_contain_common_config(&AppType::Codex, &settings, snippet),
            "TOML array subset should be detected for legacy providers"
        );

        let stripped =
            remove_common_config_from_settings(&AppType::Codex, &settings, snippet).unwrap();
        assert_eq!(stripped["auth"], json!({}));
        let stripped_config = stripped["config"].as_str().unwrap_or_default();
        let parsed = stripped_config
            .parse::<DocumentMut>()
            .expect("stripped codex config should remain valid TOML");
        let allowed_tools = parsed["allowed_tools"]
            .as_array()
            .expect("allowed_tools should remain an array");
        let values: Vec<&str> = allowed_tools
            .iter()
            .map(|value| value.as_str().expect("tool id should be string"))
            .collect();
        assert_eq!(values, vec!["tool2"]);
    }
}
