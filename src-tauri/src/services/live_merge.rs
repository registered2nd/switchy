//! Three-way merges for handing a live config back after a proxy takeover.
//!
//! `base` is what Switchy last wrote to the live file, `ours` is what Switchy
//! wants there now (the takeover backup, or a provider's settings) and
//! `theirs` is the file as it is on disk. Switchy's own changes (`base` →
//! `ours`) are applied; every change anyone else made to the file (`base` →
//! `theirs`) is kept. Where both changed the same value, the file on disk wins.

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, TableLike};

/// Merges JSON values. `None` is an absent key; a `None` result means the key
/// should be absent.
pub fn merge_json(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    if ours == base || ours == theirs {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }

    // Both sides changed it. Objects (or an object on one side and nothing on
    // the other) are merged key by key; anything else keeps the file's value.
    let empty = Map::new();
    let (Some(b), Some(o), Some(t)) = (
        json_object(base, &empty),
        json_object(ours, &empty),
        json_object(theirs, &empty),
    ) else {
        return theirs.cloned();
    };

    let mut out = t.clone();
    for key in b.keys().chain(o.keys()) {
        match merge_json(b.get(key), o.get(key), t.get(key)) {
            Some(value) => {
                out.insert(key.clone(), value);
            }
            None => {
                out.shift_remove(key);
            }
        }
    }
    if out.is_empty() && ours.is_none() {
        return None;
    }
    Some(Value::Object(out))
}

fn json_object<'a>(
    value: Option<&'a Value>,
    empty: &'a Map<String, Value>,
) -> Option<&'a Map<String, Value>> {
    match value {
        None => Some(empty),
        Some(Value::Object(map)) => Some(map),
        Some(_) => None,
    }
}

/// Merges TOML documents, keeping the formatting and comments of `theirs`.
/// Returns `None` when any of the three does not parse.
pub fn merge_toml(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let base_table: toml::Table = base.parse().ok()?;
    let ours_table: toml::Table = ours.parse().ok()?;
    let theirs_table: toml::Table = theirs.parse().ok()?;
    let ours_doc: DocumentMut = ours.parse().ok()?;
    let mut out: DocumentMut = theirs.parse().ok()?;

    merge_toml_table(
        out.as_table_mut(),
        ours_doc.as_table(),
        &base_table,
        &ours_table,
        &theirs_table,
    );
    Some(out.to_string())
}

/// `out` starts as `theirs`; `ours_items` holds the formatted items of `ours`
/// that get copied in. The `toml::Table`s are the same three documents, used
/// to compare values regardless of formatting.
fn merge_toml_table(
    out: &mut dyn TableLike,
    ours_items: &dyn TableLike,
    base: &toml::Table,
    ours: &toml::Table,
    theirs: &toml::Table,
) {
    let keys: Vec<&String> = base.keys().chain(ours.keys()).collect();
    for key in keys {
        let (b, o, t) = (base.get(key), ours.get(key), theirs.get(key));
        if o == b || o == t {
            continue;
        }
        if t == b {
            match ours_items.get(key) {
                Some(item) => {
                    out.insert(key, item.clone());
                }
                None => {
                    out.remove(key);
                }
            }
            continue;
        }

        let empty = toml::Table::new();
        let (Some(bt), Some(ot), Some(tt)) = (
            toml_table(b, &empty),
            toml_table(o, &empty),
            toml_table(t, &empty),
        ) else {
            continue;
        };
        let empty_items = toml_edit::Table::new();
        let ours_child: &dyn TableLike = ours_items
            .get(key)
            .and_then(Item::as_table_like)
            .unwrap_or(&empty_items);
        let Some(out_child) = out.get_mut(key).and_then(Item::as_table_like_mut) else {
            continue;
        };
        merge_toml_table(out_child, ours_child, bt, ot, tt);
        if o.is_none() && out_child.is_empty() {
            out.remove(key);
        }
    }
}

fn toml_table<'a>(
    value: Option<&'a toml::Value>,
    empty: &'a toml::Table,
) -> Option<&'a toml::Table> {
    match value {
        None => Some(empty),
        Some(toml::Value::Table(table)) => Some(table),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merge(base: Value, ours: Value, theirs: Value) -> Value {
        merge_json(Some(&base), Some(&ours), Some(&theirs)).expect("merged value")
    }

    #[test]
    fn keys_added_to_the_file_survive_and_switchys_own_keys_are_put_back() {
        let base = json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721", "X": "1" } });
        let ours = json!({ "env": { "X": "1" } });
        let theirs = json!({
            "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721", "X": "1" },
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "orca" }] }] }
        });
        assert_eq!(
            merge(base, ours, theirs.clone()),
            json!({ "env": { "X": "1" }, "hooks": theirs["hooks"].clone() })
        );
    }

    #[test]
    fn a_value_both_sides_changed_keeps_the_files_value() {
        let merged = merge(
            json!({ "model": "a" }),
            json!({ "model": "b" }),
            json!({ "model": "c" }),
        );
        assert_eq!(merged, json!({ "model": "c" }));
    }

    #[test]
    fn an_object_ours_removes_keeps_what_the_file_added_to_it() {
        let merged = merge(
            json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721" } }),
            json!({}),
            json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721", "FOO": "bar" } }),
        );
        assert_eq!(merged, json!({ "env": { "FOO": "bar" } }));

        let merged = merge(
            json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721" } }),
            json!({}),
            json!({ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721" } }),
        );
        assert_eq!(merged, json!({}));
    }

    #[test]
    fn toml_merge_applies_our_changes_and_keeps_the_files_other_edits() {
        let base =
            "model = \"gpt-5\"\nopenai_base_url = \"http://127.0.0.1:15721/backend-api/codex\"\n";
        let ours = "model = \"gpt-5\"\n";
        let theirs = "# my comment\nmodel = \"gpt-5\"\nopenai_base_url = \"http://127.0.0.1:15721/backend-api/codex\"\n\n[mcp_servers.orca]\ncommand = \"orca\"\n";
        let merged = merge_toml(base, ours, theirs).expect("merged");
        let table: toml::Table = merged.parse().expect("valid toml");
        assert!(table.get("openai_base_url").is_none());
        assert_eq!(
            table["mcp_servers"]["orca"]["command"].as_str(),
            Some("orca")
        );
        assert!(merged.contains("# my comment"));
    }

    #[test]
    fn toml_merge_switches_provider_tables_while_keeping_new_ones() {
        let base = "model_provider = \"a\"\n\n[model_providers.a]\nbase_url = \"http://127.0.0.1:15721/v1\"\n";
        let ours =
            "model_provider = \"b\"\n\n[model_providers.b]\nbase_url = \"https://b.example/v1\"\n";
        let theirs = "model_provider = \"a\"\n\n[model_providers.a]\nbase_url = \"http://127.0.0.1:15721/v1\"\n\n[profiles.fast]\nmodel = \"x\"\n";
        let merged = merge_toml(base, ours, theirs).expect("merged");
        let table: toml::Table = merged.parse().expect("valid toml");
        assert_eq!(table["model_provider"].as_str(), Some("b"));
        assert!(table["model_providers"].get("a").is_none());
        assert_eq!(
            table["model_providers"]["b"]["base_url"].as_str(),
            Some("https://b.example/v1")
        );
        assert_eq!(table["profiles"]["fast"]["model"].as_str(), Some("x"));
    }

    #[test]
    fn toml_merge_refuses_unparseable_input() {
        assert!(merge_toml("a = 1", "a = 2", "a = ").is_none());
    }
}
