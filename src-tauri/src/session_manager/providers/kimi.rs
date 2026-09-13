//! Kimi Code sessions: `~/.kimi-code/sessions/<workdir-slug>/<session_id>/`
//! with `state.json` (title, workDir, timestamps) and the main agent's
//! transcript at `agents/main/wire.jsonl` (one JSON event per line).

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};

const PROVIDER_ID: &str = "kimi";

pub fn sessions_root() -> PathBuf {
    crate::kimi_config::get_kimi_dir().join("sessions")
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let root = sessions_root();
    let Ok(workdirs) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut sessions = Vec::new();
    for wd in workdirs.flatten() {
        let wd_path = wd.path();
        if !wd_path.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&wd_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if dir.is_dir() && dir.join("state.json").exists() {
                if let Some(meta) = parse_session(&dir) {
                    sessions.push(meta);
                }
            }
        }
    }
    sessions
}

fn wire_path(session_dir: &Path) -> PathBuf {
    session_dir.join("agents").join("main").join("wire.jsonl")
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let wire = if path.is_dir() {
        wire_path(path)
    } else {
        path.to_path_buf()
    };
    let file = File::open(&wire).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("context.append_message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let mut content = message.get("content").map(extract_text).unwrap_or_default();
        if let Some(Value::Array(calls)) = message.get("toolCalls") {
            for call in calls {
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .or_else(|| call.get("name"))
                    .and_then(Value::as_str);
                if let Some(name) = name {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&format!("[Tool: {name}]"));
                }
            }
        }
        if content.trim().is_empty() {
            continue;
        }
        let ts = value.get("time").and_then(parse_timestamp_to_ms);
        messages.push(SessionMessage { role, content, ts });
    }

    Ok(messages)
}

pub fn delete_session(_root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        // `source_path` points at wire.jsonl: <session>/agents/main/wire.jsonl
        path.ancestors()
            .nth(3)
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("Unexpected Kimi session path: {}", path.display()))?
    };
    let meta = parse_session(&dir)
        .ok_or_else(|| format!("Failed to parse Kimi session metadata: {}", dir.display()))?;
    if meta.session_id != session_id {
        return Err(format!(
            "Kimi session ID mismatch: expected {session_id}, found {}",
            meta.session_id
        ));
    }
    std::fs::remove_dir_all(&dir)
        .map_err(|e| format!("Failed to delete Kimi session {}: {e}", dir.display()))?;
    Ok(true)
}

fn parse_session(dir: &Path) -> Option<SessionMeta> {
    let session_id = dir.file_name()?.to_str()?.to_string();
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("state.json")).ok()?).ok()?;

    let created_at = state.get("createdAt").and_then(parse_timestamp_to_ms);
    let last_active_at = state.get("updatedAt").and_then(parse_timestamp_to_ms);
    let project_dir = state
        .get("workDir")
        .and_then(Value::as_str)
        .map(str::to_string);
    let custom_title = state
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty() && *t != "New Session")
        .map(str::to_string);

    let wire = wire_path(dir);
    let first_prompt = first_user_prompt(&wire);
    let title = custom_title.or_else(|| first_prompt.as_deref().map(|s| truncate_summary(s, 160)));

    let resume_id = session_id
        .strip_prefix("session_")
        .unwrap_or(&session_id)
        .to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: title.clone(),
        summary: title,
        project_dir,
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path: Some(wire.to_string_lossy().to_string()),
        resume_command: Some(format!("kimi -S {resume_id}")),
    })
}

fn first_user_prompt(wire: &Path) -> Option<String> {
    let file = File::open(wire).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("turn.prompt") {
            continue;
        }
        let text = value.get("input").map(extract_text).unwrap_or_default();
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(root: &Path) -> PathBuf {
        let dir = root.join("wd_x_1").join("session_abc");
        std::fs::create_dir_all(dir.join("agents").join("main")).unwrap();
        std::fs::write(
            dir.join("state.json"),
            r#"{"createdAt":"2026-09-11T16:24:47.597Z","updatedAt":"2026-09-11T17:00:00.000Z","title":"New Session","workDir":"C:/Projects/x"}"#,
        )
        .unwrap();
        std::fs::write(
            wire_path(&dir),
            concat!(
                r#"{"type":"metadata","protocol_version":"1.4"}"#, "\n",
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"fix the build"}],"time":1789143888319}"#, "\n",
                r#"{"type":"context.append_message","message":{"role":"user","content":[{"type":"text","text":"fix the build"}],"toolCalls":[]},"time":1789143888320}"#, "\n",
                r#"{"type":"context.append_message","message":{"role":"assistant","content":[{"type":"text","text":"On it."}],"toolCalls":[{"function":{"name":"Shell"}}]},"time":1789143890000}"#, "\n",
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn parse_session_uses_first_prompt_as_title() {
        let temp = tempdir().unwrap();
        let dir = write_session(temp.path());
        let meta = parse_session(&dir).unwrap();
        assert_eq!(meta.session_id, "session_abc");
        assert_eq!(meta.title.as_deref(), Some("fix the build"));
        assert_eq!(meta.project_dir.as_deref(), Some("C:/Projects/x"));
        assert_eq!(meta.resume_command.as_deref(), Some("kimi -S abc"));
        assert!(meta.created_at.is_some());
    }

    #[test]
    fn load_messages_reads_appended_messages_and_tools() {
        let temp = tempdir().unwrap();
        let dir = write_session(temp.path());
        let msgs = load_messages(&wire_path(&dir)).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("[Tool: Shell]"));
    }

    #[test]
    fn delete_session_removes_the_directory() {
        let temp = tempdir().unwrap();
        let dir = write_session(temp.path());
        delete_session(temp.path(), &wire_path(&dir), "session_abc").unwrap();
        assert!(!dir.exists());
    }
}
