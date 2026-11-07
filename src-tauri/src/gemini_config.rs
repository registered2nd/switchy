use crate::config::{get_home_dir, write_text_file};
use crate::error::AppError;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Get the Gemini config directory path (honours the settings override)
pub fn get_gemini_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_gemini_override_dir() {
        return custom;
    }

    get_home_dir().join(".gemini")
}

/// Get the Gemini .env file path
pub fn get_gemini_env_path() -> PathBuf {
    get_gemini_dir().join(".env")
}

/// Parse .env file contents into key-value pairs
///
/// Lenient parser that skips invalid lines.
/// Use `parse_env_file_strict` where strict validation is needed.
pub fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();

            // Check the key is valid (non-empty; letters, digits and underscores only)
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                map.insert(key, value);
            }
        }
    }

    map
}

/// Strictly parse .env file contents, returning detailed errors
///
/// Unlike `parse_env_file`, this returns an error on an invalid line,
/// with the line number and details.
///
/// # Errors
///
/// Returns `AppError` when:
/// - a line has no `=` separator
/// - a key is empty or contains invalid characters
/// - a key does not follow environment variable naming rules
///
/// # Use cases
///
/// Reserved for future strict validation; the runtime currently uses the lenient `parse_env_file`.
/// Possible uses:
/// - validating imported config
/// - a strict mode for CLI tools
/// - diagnosing config file errors
///
/// Fully covered by tests and ready to use.
#[allow(dead_code)]
pub fn parse_env_file_strict(content: &str) -> Result<HashMap<String, String>, AppError> {
    let mut map = HashMap::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        let line_number = line_num + 1; // line numbers start at 1

        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Check for =
        if !line.contains('=') {
            return Err(AppError::localized(
                "gemini.env.parse_error.no_equals",
                format!("Invalid Gemini .env format (line {line_number}): missing '=' separator\nLine: {line}"),
            ));
        }

        // Parse KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            // Key must not be empty
            if key.is_empty() {
                return Err(AppError::localized(
                    "gemini.env.parse_error.empty_key",
                    format!("Invalid Gemini .env format (line {line_number}): variable name cannot be empty\nLine: {line}"),
                ));
            }

            // Key may contain only letters, digits and underscores
            if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(AppError::localized(
                    "gemini.env.parse_error.invalid_key",
                    format!("Invalid Gemini .env format (line {line_number}): variable name can only contain letters, numbers, and underscores\nVariable: {key}"),
                ));
            }

            map.insert(key.to_string(), value.to_string());
        }
    }

    Ok(map)
}

/// Serialize key-value pairs to .env format
pub fn serialize_env_file(map: &HashMap<String, String>) -> String {
    let mut lines = Vec::new();

    // Sort by key for stable output
    let mut keys: Vec<_> = map.keys().collect();
    keys.sort();

    for key in keys {
        if let Some(value) = map.get(key) {
            lines.push(format!("{key}={value}"));
        }
    }

    lines.join("\n")
}

/// Read the Gemini .env file
pub fn read_gemini_env() -> Result<HashMap<String, String>, AppError> {
    let path = get_gemini_env_path();

    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;

    Ok(parse_env_file(&content))
}

/// Write the Gemini .env file (atomically)
pub fn write_gemini_env_atomic(map: &HashMap<String, String>) -> Result<(), AppError> {
    let path = get_gemini_env_path();

    // Make sure the directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;

        // Set directory permissions to 700 (owner read/write/execute only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(parent)
                .map_err(|e| AppError::io(parent, e))?
                .permissions();
            perms.set_mode(0o700);
            fs::set_permissions(parent, perms).map_err(|e| AppError::io(parent, e))?;
        }
    }

    let content = serialize_env_file(map);
    write_text_file(&path, &content)?;

    // Set file permissions to 600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .map_err(|e| AppError::io(&path, e))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).map_err(|e| AppError::io(&path, e))?;
    }

    Ok(())
}

/// Convert from .env format to Provider.settings_config (JSON Value)
pub fn env_to_json(env_map: &HashMap<String, String>) -> Value {
    let mut json_map = serde_json::Map::new();

    for (key, value) in env_map {
        json_map.insert(key.clone(), Value::String(value.clone()));
    }

    serde_json::json!({ "env": json_map })
}

/// Extract .env format from Provider.settings_config (JSON Value)
pub fn json_to_env(settings: &Value) -> Result<HashMap<String, String>, AppError> {
    let mut env_map = HashMap::new();

    if let Some(env_obj) = settings.get("env").and_then(|v| v.as_object()) {
        for (key, value) in env_obj {
            if let Some(val_str) = value.as_str() {
                env_map.insert(key.clone(), val_str.to_string());
            }
        }
    }

    Ok(env_map)
}

/// Validate the basic structure of a Gemini config
///
/// Checks only the basic format and does not require GEMINI_API_KEY,
/// so a user can create a provider first and fill in the API key later.
///
/// The API key is validated when switching providers (via `validate_gemini_settings_strict`).
pub fn validate_gemini_settings(settings: &Value) -> Result<(), AppError> {
    // Basic structure only; GEMINI_API_KEY is not required
    // If there is an env field, it must be an object
    if let Some(env) = settings.get("env") {
        if !env.is_object() {
            return Err(AppError::localized(
                "gemini.validation.invalid_env",
                "Gemini config invalid: env must be an object",
            ));
        }
    }

    // If there is a config field, it must be an object or null
    if let Some(config) = settings.get("config") {
        if !(config.is_object() || config.is_null()) {
            return Err(AppError::localized(
                "gemini.validation.invalid_config",
                "Gemini config invalid: config must be an object",
            ));
        }
    }

    Ok(())
}

/// Strictly validate a Gemini config (required fields must be present)
///
/// Used when switching providers, to make sure the config has every required field.
/// For providers that need an API key (such as PackyCode), GEMINI_API_KEY is checked.
pub fn validate_gemini_settings_strict(settings: &Value) -> Result<(), AppError> {
    // Basic format validation first (including the env/config types)
    validate_gemini_settings(settings)?;

    let env_map = json_to_env(settings)?;

    // An empty env means OAuth (such as official Google); skip validation
    if env_map.is_empty() {
        return Ok(());
    }

    // A non-empty env must contain GEMINI_API_KEY
    if !env_map.contains_key("GEMINI_API_KEY") {
        return Err(AppError::localized(
            "gemini.validation.missing_api_key",
            "Gemini config missing required field: GEMINI_API_KEY",
        ));
    }

    Ok(())
}

/// Get the Gemini settings.json file path
///
/// Returns `~/.gemini/settings.json` (next to the `.env` file)
pub fn get_gemini_settings_path() -> PathBuf {
    get_gemini_dir().join("settings.json")
}

/// Update security.auth.selectedType in the Gemini directory's settings.json
///
/// This:
/// 1. reads the existing settings.json (if any)
/// 2. updates only `security.auth.selectedType`, keeping every other field
/// 3. writes the file atomically
///
/// # Parameters
/// - `selected_type`: the selectedType value to set (such as "gemini-api-key" or "oauth-personal")
fn update_selected_type(selected_type: &str) -> Result<(), AppError> {
    let settings_path = get_gemini_settings_path();

    // Make sure the directory exists
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    // Read the existing settings.json (if any)
    let mut settings_content = if settings_path.exists() {
        let content =
            fs::read_to_string(&settings_path).map_err(|e| AppError::io(&settings_path, e))?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Update only security.auth.selectedType
    if let Some(obj) = settings_content.as_object_mut() {
        let security = obj
            .entry("security")
            .or_insert_with(|| serde_json::json!({}));

        if let Some(security_obj) = security.as_object_mut() {
            let auth = security_obj
                .entry("auth")
                .or_insert_with(|| serde_json::json!({}));

            if let Some(auth_obj) = auth.as_object_mut() {
                auth_obj.insert(
                    "selectedType".to_string(),
                    Value::String(selected_type.to_string()),
                );
            }
        }
    }

    // Write the file
    crate::config::write_json_file(&settings_path, &settings_content)?;

    Ok(())
}

/// Write settings.json for the Packycode Gemini provider
///
/// Sets, in `~/.gemini/settings.json`:
/// ```json
/// {
///   "security": {
///     "auth": {
///       "selectedType": "gemini-api-key"
///     }
///   }
/// }
/// ```
///
/// Every other field in the file is kept.
pub fn write_packycode_settings() -> Result<(), AppError> {
    update_selected_type("gemini-api-key")
}

/// Write settings.json for the official Google Gemini provider (OAuth mode)
///
/// Sets, in `~/.gemini/settings.json`:
/// ```json
/// {
///   "security": {
///     "auth": {
///       "selectedType": "oauth-personal"
///     }
///   }
/// }
/// ```
///
/// Every other field in the file is kept.
pub fn write_google_oauth_settings() -> Result<(), AppError> {
    update_selected_type("oauth-personal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_file() {
        let content = r#"
# Comment line
GOOGLE_GEMINI_BASE_URL=https://example.com
GEMINI_API_KEY=sk-test123
GEMINI_MODEL=gemini-3-pro-preview

# Another comment
"#;

        let map = parse_env_file(content);

        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("GOOGLE_GEMINI_BASE_URL"),
            Some(&"https://example.com".to_string())
        );
        assert_eq!(map.get("GEMINI_API_KEY"), Some(&"sk-test123".to_string()));
        assert_eq!(
            map.get("GEMINI_MODEL"),
            Some(&"gemini-3-pro-preview".to_string())
        );
    }

    #[test]
    fn test_serialize_env_file() {
        let mut map = HashMap::new();
        map.insert("GEMINI_API_KEY".to_string(), "sk-test".to_string());
        map.insert(
            "GEMINI_MODEL".to_string(),
            "gemini-3-pro-preview".to_string(),
        );

        let content = serialize_env_file(&map);

        assert!(content.contains("GEMINI_API_KEY=sk-test"));
        assert!(content.contains("GEMINI_MODEL=gemini-3-pro-preview"));
    }

    #[test]
    fn test_env_json_conversion() {
        let mut env_map = HashMap::new();
        env_map.insert("GEMINI_API_KEY".to_string(), "test-key".to_string());

        let json = env_to_json(&env_map);
        let converted = json_to_env(&json).unwrap();

        assert_eq!(
            converted.get("GEMINI_API_KEY"),
            Some(&"test-key".to_string())
        );
    }

    #[test]
    fn test_parse_env_file_strict_success() {
        // Strict mode parses valid input
        let content = r#"
# Comment line
GOOGLE_GEMINI_BASE_URL=https://example.com
GEMINI_API_KEY=sk-test123
GEMINI_MODEL=gemini-3-pro-preview

# Another comment
"#;

        let result = parse_env_file_strict(content);
        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("GOOGLE_GEMINI_BASE_URL"),
            Some(&"https://example.com".to_string())
        );
        assert_eq!(map.get("GEMINI_API_KEY"), Some(&"sk-test123".to_string()));
        assert_eq!(
            map.get("GEMINI_MODEL"),
            Some(&"gemini-3-pro-preview".to_string())
        );
    }

    #[test]
    fn test_parse_env_file_strict_missing_equals() {
        // Strict mode rejects a line without =
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
INVALID_LINE_WITHOUT_EQUALS
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("line 2"));
        assert!(err_msg.contains("INVALID_LINE_WITHOUT_EQUALS"));
    }

    #[test]
    fn test_parse_env_file_strict_empty_key() {
        // Strict mode rejects an empty key
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
=value_without_key
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("line 2"));
        assert!(err_msg.contains("empty"));
    }

    #[test]
    fn test_parse_env_file_strict_invalid_key_characters() {
        // Strict mode rejects invalid characters (such as spaces or symbols)
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
INVALID KEY WITH SPACES=value
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("line 2"));
        assert!(err_msg.contains("INVALID KEY WITH SPACES"));
    }

    #[test]
    fn test_parse_env_file_lax_vs_strict() {
        // Lenient versus strict mode
        let content = "VALID_KEY=value
INVALID LINE
KEY_WITH-DASH=value";

        // Lenient: skip invalid lines and keep parsing
        let lax_result = parse_env_file(content);
        assert_eq!(lax_result.len(), 1); // only VALID_KEY
        assert_eq!(lax_result.get("VALID_KEY"), Some(&"value".to_string()));

        // Strict: return an error at the first invalid line
        let strict_result = parse_env_file_strict(content);
        assert!(strict_result.is_err());
    }

    #[test]
    fn test_packycode_settings_structure() {
        // Packycode settings.json has the right structure
        let settings_content = serde_json::json!({
            "security": {
                "auth": {
                    "selectedType": "gemini-api-key"
                }
            }
        });

        assert_eq!(
            settings_content["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
    }

    #[test]
    fn test_packycode_settings_merge() {
        // Merge logic keeps the other fields
        let mut existing_settings = serde_json::json!({
            "otherField": "should-be-kept",
            "security": {
                "otherSetting": "also-kept",
                "auth": {
                    "otherAuth": "preserved"
                }
            }
        });

        // Simulate updating selectedType
        if let Some(obj) = existing_settings.as_object_mut() {
            let security = obj
                .entry("security")
                .or_insert_with(|| serde_json::json!({}));

            if let Some(security_obj) = security.as_object_mut() {
                let auth = security_obj
                    .entry("auth")
                    .or_insert_with(|| serde_json::json!({}));

                if let Some(auth_obj) = auth.as_object_mut() {
                    auth_obj.insert(
                        "selectedType".to_string(),
                        Value::String("gemini-api-key".to_string()),
                    );
                }
            }
        }

        // Every field is kept
        assert_eq!(existing_settings["otherField"], "should-be-kept");
        assert_eq!(existing_settings["security"]["otherSetting"], "also-kept");
        assert_eq!(
            existing_settings["security"]["auth"]["otherAuth"],
            "preserved"
        );
        assert_eq!(
            existing_settings["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
    }

    #[test]
    fn test_google_oauth_settings_structure() {
        // Google OAuth settings.json has the right structure
        let settings_content = serde_json::json!({
            "security": {
                "auth": {
                    "selectedType": "oauth-personal"
                }
            }
        });

        assert_eq!(
            settings_content["security"]["auth"]["selectedType"],
            "oauth-personal"
        );
    }

    #[test]
    fn test_validate_empty_env_for_oauth() {
        // An empty env (official Google OAuth) passes basic validation
        let settings = serde_json::json!({
            "env": {}
        });

        assert!(validate_gemini_settings(&settings).is_ok());
        // Strict validation passes too (empty env means OAuth)
        assert!(validate_gemini_settings_strict(&settings).is_ok());
    }

    #[test]
    fn test_validate_env_with_api_key() {
        // A config with an API key passes validation
        let settings = serde_json::json!({
            "env": {
                "GEMINI_API_KEY": "sk-test123",
                "GEMINI_MODEL": "gemini-3-pro-preview"
            }
        });

        assert!(validate_gemini_settings(&settings).is_ok());
        assert!(validate_gemini_settings_strict(&settings).is_ok());
    }

    #[test]
    fn test_validate_env_without_api_key_relaxed() {
        // A non-empty config without an API key passes basic validation (the user fills it in later)
        let settings = serde_json::json!({
            "env": {
                "GEMINI_MODEL": "gemini-3-pro-preview"
            }
        });

        // Basic validation passes (the API key may come later)
        assert!(validate_gemini_settings(&settings).is_ok());
        // Strict validation fails (switching needs a complete config)
        assert!(validate_gemini_settings_strict(&settings).is_err());
    }

    #[test]
    fn test_validate_invalid_env_type() {
        // An env that is not an object fails
        let settings = serde_json::json!({
            "env": "invalid_string"
        });

        assert!(validate_gemini_settings(&settings).is_err());
    }
}
