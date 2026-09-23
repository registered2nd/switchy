//! Request body filter
//!
//! Strips private parameters that must not reach upstream, so internal data does not leak.
//!
//! ## Rules
//! - Fields starting with `_` are private and are removed recursively
//! - A whitelist can let specific `_`-prefixed fields through
//! - Nested objects and arrays are filtered at any depth
//!
//! ## Examples
//! - `_internal_id`: internal tracking ID
//! - `_debug_mode`: debug flag
//! - `_session_token`: session token
//! - `_client_version`: client version

use serde_json::Value;
use std::collections::HashSet;

/// Removes private parameters (fields starting with `_`)
///
/// Walks the JSON recursively and removes every field starting with an underscore.
///
/// # Arguments
/// * `body` - the original request body
///
/// # Returns
/// The filtered request body
///
/// # Example
/// ```ignore
/// let input = json!({
///     "model": "claude-3",
///     "_internal_id": "abc123",
///     "messages": [{"role": "user", "content": "hello", "_token": "secret"}]
/// });
/// let output = filter_private_params(input);
/// // output contains neither _internal_id nor _token
/// ```
#[cfg(test)]
pub fn filter_private_params(body: Value) -> Value {
    filter_private_params_with_whitelist(body, &[])
}

/// Removes private parameters, with a whitelist
///
/// Walks the JSON recursively and removes every field starting with an underscore,
/// except the fields named in the whitelist.
///
/// # Arguments
/// * `body` - the original request body
/// * `whitelist` - fields to keep
///
/// # Returns
/// The filtered request body
///
/// # Example
/// ```ignore
/// let input = json!({
///     "model": "claude-3",
///     "_metadata": {"key": "value"},  // whitelisted, kept
///     "_internal_id": "abc123"        // not whitelisted, removed
/// });
/// let output = filter_private_params_with_whitelist(input, &["_metadata"]);
/// // output contains _metadata but not _internal_id
/// ```
pub fn filter_private_params_with_whitelist(body: Value, whitelist: &[String]) -> Value {
    let whitelist_set: HashSet<&str> = whitelist.iter().map(|s| s.as_str()).collect();
    filter_recursive_with_whitelist(body, &mut Vec::new(), &whitelist_set)
}

/// Recursive filter (with whitelist)
fn filter_recursive_with_whitelist(
    value: Value,
    removed_keys: &mut Vec<String>,
    whitelist: &HashSet<&str>,
) -> Value {
    match value {
        Value::Object(map) => {
            let filtered: serde_json::Map<String, Value> = map
                .into_iter()
                .filter_map(|(key, val)| {
                    // Remove fields that start with _ and are not whitelisted
                    if key.starts_with('_') && !whitelist.contains(key.as_str()) {
                        removed_keys.push(key);
                        None
                    } else {
                        Some((
                            key,
                            filter_recursive_with_whitelist(val, removed_keys, whitelist),
                        ))
                    }
                })
                .collect();

            // Log only when something was removed (not on every request)
            if !removed_keys.is_empty() {
                log::debug!("[BodyFilter] Removed private parameters: {removed_keys:?}");
                removed_keys.clear();
            }

            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| filter_recursive_with_whitelist(v, removed_keys, whitelist))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_filter_top_level_private_params() {
        let input = json!({
            "model": "claude-3",
            "_internal_id": "abc123",
            "_debug": true,
            "max_tokens": 1024
        });

        let output = filter_private_params(input);

        assert!(output.get("model").is_some());
        assert!(output.get("max_tokens").is_some());
        assert!(output.get("_internal_id").is_none());
        assert!(output.get("_debug").is_none());
    }

    #[test]
    fn test_filter_nested_private_params() {
        let input = json!({
            "model": "claude-3",
            "messages": [
                {
                    "role": "user",
                    "content": "hello",
                    "_session_token": "secret"
                }
            ],
            "metadata": {
                "user_id": "user-1",
                "_tracking_id": "track-1"
            }
        });

        let output = filter_private_params(input);

        // Top-level fields kept
        assert!(output.get("model").is_some());
        assert!(output.get("messages").is_some());
        assert!(output.get("metadata").is_some());

        // Private parameters inside the messages array removed
        let messages = output.get("messages").unwrap().as_array().unwrap();
        assert!(messages[0].get("role").is_some());
        assert!(messages[0].get("content").is_some());
        assert!(messages[0].get("_session_token").is_none());

        // Private parameters inside the metadata object removed
        let metadata = output.get("metadata").unwrap();
        assert!(metadata.get("user_id").is_some());
        assert!(metadata.get("_tracking_id").is_none());
    }

    #[test]
    fn test_filter_deeply_nested() {
        let input = json!({
            "level1": {
                "level2": {
                    "level3": {
                        "keep": "value",
                        "_remove": "secret"
                    }
                }
            }
        });

        let output = filter_private_params(input);

        let level3 = output
            .get("level1")
            .unwrap()
            .get("level2")
            .unwrap()
            .get("level3")
            .unwrap();

        assert!(level3.get("keep").is_some());
        assert!(level3.get("_remove").is_none());
    }

    #[test]
    fn test_filter_array_of_objects() {
        let input = json!({
            "items": [
                {"id": 1, "_secret": "a"},
                {"id": 2, "_secret": "b"},
                {"id": 3, "_secret": "c"}
            ]
        });

        let output = filter_private_params(input);
        let items = output.get("items").unwrap().as_array().unwrap();

        for item in items {
            assert!(item.get("id").is_some());
            assert!(item.get("_secret").is_none());
        }
    }

    #[test]
    fn test_no_private_params() {
        let input = json!({
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hello"}]
        });

        let output = filter_private_params(input.clone());

        // With no private parameters, output equals input
        assert_eq!(input, output);
    }

    #[test]
    fn test_empty_object() {
        let input = json!({});
        let output = filter_private_params(input);
        assert_eq!(output, json!({}));
    }

    #[test]
    fn test_primitive_values() {
        // Primitive values are left unchanged
        assert_eq!(filter_private_params(json!(42)), json!(42));
        assert_eq!(filter_private_params(json!("string")), json!("string"));
        assert_eq!(filter_private_params(json!(true)), json!(true));
        assert_eq!(filter_private_params(json!(null)), json!(null));
    }

    #[test]
    fn test_whitelist_preserves_private_params() {
        let input = json!({
            "model": "claude-3",
            "_metadata": {"key": "value"},
            "_internal_id": "abc123",
            "_stream_options": {"include_usage": true}
        });

        let whitelist = vec!["_metadata".to_string(), "_stream_options".to_string()];
        let output = filter_private_params_with_whitelist(input, &whitelist);

        // Whitelisted fields kept
        assert!(output.get("_metadata").is_some());
        assert!(output.get("_stream_options").is_some());
        // Private fields not on the whitelist removed
        assert!(output.get("_internal_id").is_none());
        // Ordinary fields kept
        assert!(output.get("model").is_some());
    }

    #[test]
    fn test_whitelist_nested() {
        let input = json!({
            "data": {
                "_allowed": "keep",
                "_forbidden": "remove",
                "normal": "value"
            }
        });

        let whitelist = vec!["_allowed".to_string()];
        let output = filter_private_params_with_whitelist(input, &whitelist);

        let data = output.get("data").unwrap();
        assert!(data.get("_allowed").is_some());
        assert!(data.get("_forbidden").is_none());
        assert!(data.get("normal").is_some());
    }

    #[test]
    fn test_empty_whitelist_same_as_default() {
        let input = json!({
            "model": "claude-3",
            "_internal_id": "abc123"
        });

        let output1 = filter_private_params(input.clone());
        let output2 = filter_private_params_with_whitelist(input, &[]);

        assert_eq!(output1, output2);
    }
}
