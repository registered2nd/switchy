// Repair Claude Code session JSONLs whose `thinking` / `redacted_thinking`
// content blocks were produced by an upstream that the new endpoint can't
// validate (empty signatures from wrapping relays, invalid `data` in
// redacted_thinking). Strips the offending blocks and rewrites parentUuid
// chains so resume reconstructs the conversation graph end-to-end.
//
// See LEARNINGS.md "Claude Code transcript repair after provider-switch
// failures" for the why.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::Value;

use crate::config::get_claude_config_dir;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenSessionInfo {
    pub session_id: String,
    pub source_path: String,
    pub project_dir: Option<String>,
    pub last_modified_ms: i64,
    pub total_lines: usize,
    pub thinking_blocks: usize,
    pub empty_signatures: usize,
    pub redacted_thinking_blocks: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub source_path: String,
    pub backup_path: String,
    pub lines_before: usize,
    pub lines_after: usize,
    pub thinking_dropped: usize,
    pub redacted_thinking_dropped: usize,
    pub parent_uuid_rewrites: usize,
}

pub fn scan_broken_sessions() -> Vec<BrokenSessionInfo> {
    let root = get_claude_config_dir().join("projects");
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);

    let mut results = Vec::new();
    for path in files {
        if is_backup_file(&path) {
            continue;
        }
        if let Some(info) = inspect_session(&path) {
            if info.thinking_blocks > 0 || info.redacted_thinking_blocks > 0 {
                results.push(info);
            }
        }
    }
    // newest first
    results.sort_by(|a, b| b.last_modified_ms.cmp(&a.last_modified_ms));
    results
}

pub fn repair_session(path: &Path) -> Result<RepairResult, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let backup_path = backup_path_for(path);
    fs::copy(path, &backup_path).map_err(|e| {
        format!(
            "Failed to back up {} -> {}: {e}",
            path.display(),
            backup_path.display()
        )
    })?;

    let mut lines_before = 0usize;
    let mut thinking_dropped = 0usize;
    let mut redacted_thinking_dropped = 0usize;
    let mut parent_uuid_rewrites = 0usize;

    // remap[dropped_uuid] = surviving parentUuid (None means root / no parent)
    let mut remap: HashMap<String, Option<String>> = HashMap::new();
    let mut output: Vec<String> = Vec::new();

    for raw in content.split('\n') {
        if raw.trim().is_empty() {
            output.push(raw.to_string());
            continue;
        }
        lines_before += 1;

        let mut obj: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => {
                output.push(raw.to_string());
                continue;
            }
        };

        let block_summary = summarize_content_blocks(&obj);

        match block_summary {
            BlockSummary::OnlyThinking { thinking, redacted } => {
                if let Some(uuid) = obj.get("uuid").and_then(Value::as_str).map(String::from) {
                    let parent = obj
                        .get("parentUuid")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let resolved = resolve_remap(&remap, parent);
                    remap.insert(uuid, resolved);
                }
                thinking_dropped += thinking;
                redacted_thinking_dropped += redacted;
                continue; // drop line
            }
            BlockSummary::Mixed { thinking, redacted } => {
                strip_thinking_blocks(&mut obj);
                thinking_dropped += thinking;
                redacted_thinking_dropped += redacted;
                if rewrite_parent_uuid(&mut obj, &remap) {
                    parent_uuid_rewrites += 1;
                }
                let serialized = serde_json::to_string(&obj)
                    .map_err(|e| format!("Failed to re-serialize line: {e}"))?;
                output.push(serialized);
            }
            BlockSummary::None => {
                if rewrite_parent_uuid(&mut obj, &remap) {
                    parent_uuid_rewrites += 1;
                    let serialized = serde_json::to_string(&obj)
                        .map_err(|e| format!("Failed to re-serialize line: {e}"))?;
                    output.push(serialized);
                } else {
                    output.push(raw.to_string());
                }
            }
        }
    }

    let lines_after = output.iter().filter(|s| !s.trim().is_empty()).count();

    fs::write(path, output.join("\n"))
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    Ok(RepairResult {
        source_path: path.to_string_lossy().to_string(),
        backup_path: backup_path.to_string_lossy().to_string(),
        lines_before,
        lines_after,
        thinking_dropped,
        redacted_thinking_dropped,
        parent_uuid_rewrites,
    })
}

enum BlockSummary {
    None,
    OnlyThinking { thinking: usize, redacted: usize },
    Mixed { thinking: usize, redacted: usize },
}

fn summarize_content_blocks(obj: &Value) -> BlockSummary {
    let blocks = match obj
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    {
        Some(arr) => arr,
        None => return BlockSummary::None,
    };
    if blocks.is_empty() {
        return BlockSummary::None;
    }
    let mut thinking = 0usize;
    let mut redacted = 0usize;
    let mut other = 0usize;
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("thinking") => thinking += 1,
            Some("redacted_thinking") => redacted += 1,
            _ => other += 1,
        }
    }
    if thinking == 0 && redacted == 0 {
        BlockSummary::None
    } else if other == 0 {
        BlockSummary::OnlyThinking { thinking, redacted }
    } else {
        BlockSummary::Mixed { thinking, redacted }
    }
}

fn strip_thinking_blocks(obj: &mut Value) {
    let Some(content) = obj
        .get_mut("message")
        .and_then(|m| m.get_mut("content"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    content.retain(|b| {
        let t = b.get("type").and_then(Value::as_str);
        t != Some("thinking") && t != Some("redacted_thinking")
    });
}

fn rewrite_parent_uuid(obj: &mut Value, remap: &HashMap<String, Option<String>>) -> bool {
    let Some(parent) = obj.get("parentUuid").and_then(Value::as_str) else {
        return false;
    };
    let Some(replacement) = remap.get(parent) else {
        return false;
    };
    let new_value = match replacement {
        Some(p) => Value::String(p.clone()),
        None => Value::Null,
    };
    obj["parentUuid"] = new_value;
    true
}

fn resolve_remap(
    remap: &HashMap<String, Option<String>>,
    mut current: Option<String>,
) -> Option<String> {
    // Follow chains of dropped uuids to their first surviving ancestor.
    while let Some(ref u) = current {
        match remap.get(u) {
            Some(next) => current = next.clone(),
            None => break,
        }
    }
    current
}

fn inspect_session(path: &Path) -> Option<BrokenSessionInfo> {
    let content = fs::read_to_string(path).ok()?;

    let mut total_lines = 0usize;
    let mut thinking_blocks = 0usize;
    let mut empty_signatures = 0usize;
    let mut redacted_thinking_blocks = 0usize;
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;

    for line in content.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        total_lines += 1;
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if session_id.is_none() {
            session_id = v
                .get("sessionId")
                .and_then(Value::as_str)
                .map(String::from);
        }
        if project_dir.is_none() {
            project_dir = v.get("cwd").and_then(Value::as_str).map(String::from);
        }
        if let Some(blocks) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("thinking") => {
                        thinking_blocks += 1;
                        if b.get("signature").and_then(Value::as_str) == Some("") {
                            empty_signatures += 1;
                        }
                    }
                    Some("redacted_thinking") => {
                        redacted_thinking_blocks += 1;
                    }
                    _ => {}
                }
            }
        }
    }

    let last_modified_ms = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(String::from)
    })?;

    Some(BrokenSessionInfo {
        session_id,
        source_path: path.to_string_lossy().to_string(),
        project_dir,
        last_modified_ms,
        total_lines,
        thinking_blocks,
        empty_signatures,
        redacted_thinking_blocks,
    })
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn is_backup_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".bak.jsonl"))
        .unwrap_or(false)
}

fn backup_path_for(path: &Path) -> PathBuf {
    // foo.jsonl -> foo.bak.jsonl
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    parent.join(format!("{stem}.bak.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(path: &Path, lines: &[&str]) {
        fs::write(path, lines.join("\n")).expect("write session");
    }

    #[test]
    fn drops_thinking_only_lines_and_rewrites_chain() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("s.jsonl");
        write_session(
            &path,
            &[
                r#"{"sessionId":"s1","cwd":"/tmp","uuid":"u1","timestamp":"2026-05-04T10:00:00Z"}"#,
                r#"{"uuid":"u2","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x","signature":""}]}}"#,
                r#"{"uuid":"u3","parentUuid":"u2","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
            ],
        );

        let result = repair_session(&path).expect("repair");

        assert_eq!(result.thinking_dropped, 1);
        assert_eq!(result.redacted_thinking_dropped, 0);
        assert_eq!(result.parent_uuid_rewrites, 1);

        let after = fs::read_to_string(&path).expect("read");
        // u2 line dropped
        assert!(!after.contains("\"uuid\":\"u2\""));
        // u3's parentUuid rewritten u2 -> u1
        assert!(after.contains("\"parentUuid\":\"u1\""));
        // u1 line untouched
        assert!(after.contains("\"sessionId\":\"s1\""));

        // backup exists with original 3-line content
        let bak = tmp.path().join("s.bak.jsonl");
        let bak_content = fs::read_to_string(&bak).expect("read bak");
        assert!(bak_content.contains("u2"));
    }

    #[test]
    fn strips_mixed_content_blocks_keeps_others() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("s.jsonl");
        write_session(
            &path,
            &[r#"{"uuid":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x","signature":""},{"type":"text","text":"kept"}]}}"#],
        );

        let result = repair_session(&path).expect("repair");
        assert_eq!(result.thinking_dropped, 1);

        let after = fs::read_to_string(&path).expect("read");
        assert!(after.contains("\"text\":\"kept\""));
        assert!(!after.contains("\"type\":\"thinking\""));
    }

    #[test]
    fn handles_redacted_thinking() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("s.jsonl");
        write_session(
            &path,
            &[
                r#"{"uuid":"u1","message":{"role":"user","content":[{"type":"text","text":"q"}]}}"#,
                r#"{"uuid":"u2","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"redacted_thinking","data":"xxx"}]}}"#,
                r#"{"uuid":"u3","parentUuid":"u2","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
            ],
        );

        let result = repair_session(&path).expect("repair");
        assert_eq!(result.redacted_thinking_dropped, 1);
        assert_eq!(result.parent_uuid_rewrites, 1);

        let after = fs::read_to_string(&path).expect("read");
        assert!(!after.contains("redacted_thinking"));
        assert!(after.contains("\"parentUuid\":\"u1\""));
    }

    #[test]
    fn transitive_remap_through_chain_of_dropped_lines() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("s.jsonl");
        write_session(
            &path,
            &[
                r#"{"uuid":"u1","message":{"role":"user","content":[{"type":"text","text":"q"}]}}"#,
                r#"{"uuid":"u2","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":""}]}}"#,
                r#"{"uuid":"u3","parentUuid":"u2","message":{"role":"assistant","content":[{"type":"redacted_thinking","data":"xxx"}]}}"#,
                r#"{"uuid":"u4","parentUuid":"u3","message":{"role":"assistant","content":[{"type":"text","text":"final"}]}}"#,
            ],
        );

        let result = repair_session(&path).expect("repair");
        assert_eq!(result.thinking_dropped, 1);
        assert_eq!(result.redacted_thinking_dropped, 1);
        assert_eq!(result.parent_uuid_rewrites, 1);

        let after = fs::read_to_string(&path).expect("read");
        // u4's parent traversed u3 -> u2 -> u1 (both dropped)
        assert!(after.contains("\"uuid\":\"u4\""));
        assert!(after.contains("\"parentUuid\":\"u1\""));
    }

    #[test]
    fn scan_skips_backup_files() {
        // Smoke: backup_path_for produces the right name
        let p = Path::new("/foo/abc-123.jsonl");
        assert_eq!(
            backup_path_for(p),
            PathBuf::from("/foo/abc-123.bak.jsonl")
        );
    }
}
