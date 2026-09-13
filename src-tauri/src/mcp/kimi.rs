//! Kimi Code MCP sync and import — `~/.kimi-code/mcp.json`, a plain
//! `{ "mcpServers": { id: spec } }` file like Claude's.

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::config::{read_json_file, write_json_file};
use crate::error::AppError;

use super::validation::{extract_server_spec, validate_server_spec};

/// Kimi not installed / never run: leave the directory alone.
fn should_sync_kimi_mcp() -> bool {
    crate::kimi_config::get_kimi_dir().exists()
}

fn read_mcp_servers_map() -> Result<HashMap<String, Value>, AppError> {
    let path = crate::kimi_config::get_kimi_mcp_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let root: Value = read_json_file(&path)?;
    Ok(root
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default())
}

fn set_mcp_servers_map(servers: &HashMap<String, Value>) -> Result<(), AppError> {
    let path = crate::kimi_config::get_kimi_mcp_path();
    let mut root: Value = if path.exists() {
        read_json_file(&path)?
    } else {
        json!({})
    };
    if !root.is_object() {
        root = json!({});
    }
    let map: serde_json::Map<String, Value> = servers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    root.as_object_mut()
        .expect("root is object")
        .insert("mcpServers".to_string(), Value::Object(map));
    write_json_file(&path, &root)
}

/// Import Kimi's `mcp.json` into the unified store; existing servers just gain
/// the Kimi flag.
pub fn import_from_kimi(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let map = read_mcp_servers_map()?;
    if map.is_empty() {
        return Ok(0);
    }

    let servers = config.mcp.servers.get_or_insert_with(HashMap::new);
    let mut changed = 0;
    for (id, spec) in map {
        if let Err(err) = validate_server_spec(&spec) {
            log::warn!("跳过无效的 Kimi MCP 条目 '{id}': {err}");
            continue;
        }
        match servers.get_mut(&id) {
            Some(existing) => {
                if !existing.apps.kimi {
                    existing.apps.kimi = true;
                    changed += 1;
                }
            }
            None => {
                servers.insert(
                    id.clone(),
                    McpServer {
                        id: id.clone(),
                        name: id.clone(),
                        server: spec,
                        apps: McpApps {
                            kimi: true,
                            ..Default::default()
                        },
                        description: None,
                        homepage: None,
                        docs: None,
                        tags: Vec::new(),
                    },
                );
                changed += 1;
            }
        }
    }
    Ok(changed)
}

pub fn sync_single_server_to_kimi(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    if !should_sync_kimi_mcp() {
        return Ok(());
    }
    let spec = extract_server_spec(server_spec)?;
    let mut current = read_mcp_servers_map()?;
    current.insert(id.to_string(), spec);
    set_mcp_servers_map(&current)
}

pub fn remove_server_from_kimi(id: &str) -> Result<(), AppError> {
    if !should_sync_kimi_mcp() {
        return Ok(());
    }
    let mut current = read_mcp_servers_map()?;
    if current.remove(id).is_none() {
        return Ok(());
    }
    set_mcp_servers_map(&current)
}
