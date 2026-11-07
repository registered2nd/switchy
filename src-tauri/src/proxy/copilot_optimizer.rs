//! Copilot request optimizer
//!
//! Fixes inflated usage through the GitHub Copilot proxy (issue #1813).
//!
//! Copilot uses the `x-initiator` header to tell user-initiated requests from agent continuations:
//! - `user`: counts as a premium interaction (consumes quota)
//! - `agent`: treated as a continuation of the previous interaction (no extra charge)
//!
//! Reference implementation: https://github.com/caozhiyuan/copilot-api

use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Request classification result
#[derive(Debug, Clone)]
pub struct CopilotClassification {
    /// "user" or "agent" — mapped to the x-initiator header
    pub initiator: &'static str,
    /// Whether this is a warmup/probe request (can be downgraded to a small model)
    pub is_warmup: bool,
    /// Whether this is a context compaction request
    pub is_compact: bool,
}

/// Classifies an Anthropic-format request body to decide the Copilot headers.
///
/// Algorithm (looks only at the last message, matching the reference implementation caozhiyuan/copilot-api):
/// 1. No messages → "user" (safe default, first request)
/// 2. Last message role=user:
///    - content has a block that is not tool_result → "user"
///    - content is all tool_result → "agent"
///    - matches the compact pattern → "agent"
/// 3. Last message role is not user → "user" (safe default)
///
/// Warmup detection (matching the reference implementation):
/// - `anthropic-beta` header present + no tools + not compact → warmup
///
/// `compact_detection`: whether compact detection is on. When false it is skipped,
/// so the `CopilotOptimizerConfig.compact_detection` switch actually takes effect.
pub fn classify_request(
    body: &Value,
    has_anthropic_beta: bool,
    compact_detection: bool,
) -> CopilotClassification {
    let is_compact = compact_detection && is_compact_request(body);

    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(msgs) if !msgs.is_empty() => msgs,
        _ => {
            return CopilotClassification {
                initiator: "user",
                is_warmup: is_warmup_request(body, has_anthropic_beta, false),
                is_compact: false,
            }
        }
    };

    let last_msg = &messages[messages.len() - 1];
    let role = last_msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

    // Only role=user messages need further classification
    if role != "user" {
        return CopilotClassification {
            initiator: "user",
            is_warmup: false,
            is_compact,
        };
    }

    // Reference implementation logic (Messages API path):
    // if content is an array, check for a block that is not tool_result:
    // any → "user"; all tool_result → "agent"
    // if content is a string → "user"
    let is_user_initiated = match last_msg.get("content") {
        Some(content) if content.is_array() => {
            let blocks = content.as_array().unwrap();
            // A non-tool_result block exists → user-initiated
            blocks
                .iter()
                .any(|block| block.get("type").and_then(|t| t.as_str()) != Some("tool_result"))
        }
        Some(content) if content.is_string() => true,
        _ => false,
    };

    let initiator = if !is_user_initiated || is_compact {
        "agent"
    } else {
        "user"
    };

    CopilotClassification {
        initiator,
        is_warmup: initiator == "user" && is_warmup_request(body, has_anthropic_beta, is_compact),
        is_compact,
    }
}

/// Detects warmup/probe requests (suitable for downgrading to a small model).
///
/// Matching the reference implementation, all three conditions must hold:
/// 1. The `anthropic-beta` header is present (the mark of a Claude Code warmup probe)
/// 2. No tools defined
/// 3. Not a compact request
fn is_warmup_request(body: &Value, has_anthropic_beta: bool, is_compact: bool) -> bool {
    if !has_anthropic_beta || is_compact {
        return false;
    }
    // No tools defined
    !matches!(body.get("tools"), Some(tools) if tools.is_array() && !tools.as_array().unwrap().is_empty())
}

/// Detects Claude Code context compaction (compact) requests.
///
/// Matches only machine markers **generated internally** by Claude Code, not generic phrases a user might type,
/// so real user requests are not mislabelled as agent.
///
/// Strong signals:
/// 1. system prompt — Claude Code compact mode sets a dedicated system prompt the user cannot set
/// 2. "CRITICAL: Respond with TEXT ONLY. Do NOT call any tools." — a machine instruction
/// 3. Both "Pending Tasks:" and "Current Work:" present — Claude Code compact structure markers
fn is_compact_request(body: &Value) -> bool {
    // Signal 1: the system prompt starts with the Claude Code compact prefix
    // Users cannot control the system prompt directly in Claude Code, so this is the most reliable signal
    if let Some(system) = body.get("system") {
        let system_text = if let Some(s) = system.as_str() {
            s.to_string()
        } else if let Some(arr) = system.as_array() {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        };

        if system_text
            .starts_with("You are a helpful AI assistant tasked with summarizing conversations")
        {
            return true;
        }
    }

    // Signals 2 & 3: check the last user message for machine-generated markers
    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(msgs) => msgs,
        None => return false,
    };

    if let Some(last_msg) = messages.last() {
        if last_msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            return false;
        }

        let text = extract_text_from_message(last_msg);

        // Signal 2: Claude Code compact machine instruction (case-sensitive, exact match)
        if text.contains("CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.") {
            return true;
        }

        // Signal 3: Claude Code compact structure markers (both must appear)
        if text.contains("Pending Tasks:") && text.contains("Current Work:") {
            return true;
        }
    }

    false
}

/// Merges tool_result and text blocks in user messages.
///
/// Matches the reference implementation `mergeToolResultForClaude`:
///
/// **Within a message** (the core): inside one user message, text blocks are absorbed into tool_result blocks,
/// leaving only tool_result blocks, so Copilot does not count it as a user-initiated interaction.
///
/// Why: for skill calls, edit hooks, plan reminders and similar, Claude Code sends user messages that mix
/// tool_result + text. The text block makes Copilot count it as a premium request.
///
/// **Across messages** (supplementary): consecutive tool_result-only user messages are merged into one.
pub fn merge_tool_results(mut body: Value) -> Value {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(msgs) if !msgs.is_empty() => msgs,
        _ => return body,
    };

    // Phase 1: within-message merge — absorb text blocks into tool_result blocks
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(blocks) => blocks,
            None => continue,
        };

        // Separate tool_result and text blocks
        let mut tool_results: Vec<Value> = Vec::new();
        let mut text_blocks: Vec<Value> = Vec::new();
        let mut valid = true;

        for block in content {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("tool_result") => tool_results.push(block.clone()),
                Some("text") => text_blocks.push(block.clone()),
                _ => {
                    // Some other block type → skip this message
                    valid = false;
                    break;
                }
            }
        }

        // Merge only when both tool_result and text are present
        if !valid || tool_results.is_empty() || text_blocks.is_empty() {
            continue;
        }

        // Merge strategy (matching the reference implementation)
        let merged = merge_blocks_into_tool_results(tool_results, text_blocks);
        msg["content"] = Value::Array(merged);
    }

    // Phase 2: cross-message merge — merge consecutive tool_result-only user messages
    let messages = body["messages"].as_array().unwrap().clone();
    if messages.len() <= 1 {
        return body;
    }

    let mut merged_msgs: Vec<Value> = Vec::with_capacity(messages.len());
    let mut i = 0;

    while i < messages.len() {
        if is_tool_result_only_message(&messages[i]) {
            let mut combined_content: Vec<Value> = Vec::new();
            while i < messages.len() && is_tool_result_only_message(&messages[i]) {
                if let Some(content) = messages[i].get("content").and_then(|c| c.as_array()) {
                    combined_content.extend(content.iter().cloned());
                }
                i += 1;
            }
            if !combined_content.is_empty() {
                merged_msgs.push(serde_json::json!({
                    "role": "user",
                    "content": combined_content
                }));
            }
        } else {
            merged_msgs.push(messages[i].clone());
            i += 1;
        }
    }

    body["messages"] = Value::Array(merged_msgs);
    body
}

/// Generates a deterministic request ID from the last user message.
///
/// Matching the reference implementation:
/// - Hash input: sessionId + lastUserContent (excluding tool_result and cache_control)
/// - Falls back to a random UUID when there is no user content
/// - UUID v4 format
pub fn deterministic_request_id(body: &Value, session_id: &str) -> String {
    let last_user_content = find_last_user_content(body);

    match last_user_content {
        Some(content) => {
            let mut hasher = Sha256::new();
            hasher.update(session_id.as_bytes());
            hasher.update(content.as_bytes());
            let result = hasher.finalize();

            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&result[..16]);
            // UUID v4 version and variant bits (as in the reference implementation)
            bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
            bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1

            Uuid::from_bytes(bytes).to_string()
        }
        None => Uuid::new_v4().to_string(),
    }
}

// ─── Internal helpers ─────────────────────────────────

/// Finds the non-tool_result content of the last user message.
///
/// Matches the reference implementation's `findLastUserContent`:
/// - walks messages from the end
/// - excludes tool_result blocks
/// - excludes the cache_control field
fn find_last_user_content(body: &Value) -> Option<String> {
    let messages = body.get("messages").and_then(|m| m.as_array())?;

    for msg in messages.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let content = msg.get("content")?;

        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }

        if let Some(blocks) = content.as_array() {
            // Drop tool_result, keep other blocks (without cache_control)
            let filtered: Vec<Value> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) != Some("tool_result"))
                .map(|b| {
                    let mut b = b.clone();
                    if let Some(obj) = b.as_object_mut() {
                        obj.remove("cache_control");
                    }
                    b
                })
                .collect();

            if !filtered.is_empty() {
                return Some(serde_json::to_string(&filtered).unwrap_or_default());
            }
        }
    }

    None
}

/// Merges text blocks into tool_result blocks.
///
/// Two strategies (matching the reference implementation):
/// - Equal counts: paired one to one; each text is appended to its tool_result's content
/// - Unequal counts: all text is appended to the last tool_result's content
fn merge_blocks_into_tool_results(
    mut tool_results: Vec<Value>,
    text_blocks: Vec<Value>,
) -> Vec<Value> {
    if tool_results.len() == text_blocks.len() {
        // Pair one to one
        for (tr, tb) in tool_results.iter_mut().zip(text_blocks.iter()) {
            append_text_to_tool_result(tr, tb);
        }
    } else {
        // Append all text to the last tool_result
        if let Some(last_tr) = tool_results.last_mut() {
            for tb in &text_blocks {
                append_text_to_tool_result(last_tr, tb);
            }
        }
    }
    tool_results
}

/// Appends a text block's content to a tool_result's content
fn append_text_to_tool_result(tool_result: &mut Value, text_block: &Value) {
    let text = text_block
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if text.trim().is_empty() {
        return;
    }

    // A tool_result's content may be a string or an array
    match tool_result.get("content") {
        Some(c) if c.is_string() => {
            let existing = c.as_str().unwrap_or("");
            tool_result["content"] = Value::String(format!("{existing}\n{text}"));
        }
        Some(c) if c.is_array() => {
            let arr = tool_result["content"].as_array_mut().unwrap();
            arr.push(serde_json::json!({"type": "text", "text": text}));
        }
        _ => {
            // content missing or null — set it directly
            tool_result["content"] = Value::String(text.to_string());
        }
    }
}

/// Extracts the text content from a message
fn extract_text_from_message(msg: &Value) -> String {
    match msg.get("content") {
        Some(content) if content.is_string() => content.as_str().unwrap_or("").to_string(),
        Some(content) if content.is_array() => {
            let blocks = content.as_array().unwrap();
            blocks
                .iter()
                .filter_map(|block| {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        block.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => String::new(),
    }
}

/// Whether a message is a tool_result-only user message
fn is_tool_result_only_message(msg: &Value) -> bool {
    if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
        return false;
    }
    match msg.get("content").and_then(|c| c.as_array()) {
        Some(blocks) if !blocks.is_empty() => blocks
            .iter()
            .all(|block| block.get("type").and_then(|t| t.as_str()) == Some("tool_result")),
        _ => false,
    }
}

// ─── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // === classify_request tests ===

    #[test]
    fn test_classify_user_text_message() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "Hello, please help me write some code"}
            ]
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "user");
        assert!(!result.is_compact);
    }

    #[test]
    fn test_classify_user_text_array_message() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Please explain this code"}
                ]}
            ]
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "user");
    }

    #[test]
    fn test_classify_tool_result_only() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "tools": [{"name": "Read", "description": "Read a file", "input_schema": {}}],
            "messages": [
                {"role": "user", "content": "Read the file"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "I'll read that file."},
                    {"type": "tool_use", "id": "toolu_123", "name": "Read", "input": {"path": "/tmp/test.rs"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_123", "content": "file contents here"}
                ]}
            ]
        });
        let result = classify_request(&body, true, true);
        assert_eq!(result.initiator, "agent");
        assert!(!result.is_warmup);
    }

    #[test]
    fn test_classify_tool_result_with_text_block() {
        // Key scenario from the reference implementation: tool_result + text block
        // A non-tool_result block exists → still "user"
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_123", "content": "file contents"},
                    {"type": "text", "text": "Now please refactor this code"}
                ]}
            ]
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "user");
    }

    #[test]
    fn test_classify_empty_messages() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": []
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "user");
    }

    #[test]
    fn test_classify_no_messages() {
        let body = json!({"model": "claude-sonnet-4-20250514"});
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "user");
    }

    #[test]
    fn test_classify_compact_request_system_prompt() {
        // compact detected via the strong system prompt signal
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "system": "You are a helpful AI assistant tasked with summarizing conversations. Please create a summary.",
            "messages": [
                {"role": "user", "content": "Here is the conversation history to summarize..."}
            ]
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "agent");
        assert!(result.is_compact);
    }

    #[test]
    fn test_classify_compact_request_critical_marker() {
        // compact detected via the CRITICAL machine instruction
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "CRITICAL: Respond with TEXT ONLY. Do NOT call any tools. Summarize the conversation."}
                ]}
            ]
        });
        let result = classify_request(&body, false, true);
        assert_eq!(result.initiator, "agent");
        assert!(result.is_compact);
    }

    #[test]
    fn test_classify_compact_disabled_by_config() {
        // With compact_detection=false, matching content is not marked compact
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "system": "You are a helpful AI assistant tasked with summarizing conversations.",
            "messages": [
                {"role": "user", "content": "Summarize"}
            ]
        });
        let result = classify_request(&body, false, false); // compact_detection=false
        assert_eq!(result.initiator, "user"); // not marked as agent
        assert!(!result.is_compact);
    }

    #[test]
    fn test_no_false_positive_on_user_summarize_request() {
        // P1 fix check: a user typing "summarize the conversation" must not be taken for compact
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "Please summarize the conversation so far into a concise summary."}
            ]
        });
        let result = classify_request(&body, false, true);
        // No strong system prompt signal and no CRITICAL instruction → not compact → user
        assert_eq!(result.initiator, "user");
        assert!(!result.is_compact);
    }

    // === warmup tests (matching the reference implementation) ===

    #[test]
    fn test_warmup_with_anthropic_beta_no_tools() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        // has_anthropic_beta=true, no tools → warmup
        let result = classify_request(&body, true, true);
        assert!(result.is_warmup);
    }

    #[test]
    fn test_not_warmup_without_anthropic_beta() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        // has_anthropic_beta=false → not warmup
        let result = classify_request(&body, false, true);
        assert!(!result.is_warmup);
    }

    #[test]
    fn test_not_warmup_with_tools() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "tools": [{"name": "Read", "description": "Read a file", "input_schema": {}}],
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        // Has tools → not warmup (even with anthropic-beta)
        let result = classify_request(&body, true, true);
        assert!(!result.is_warmup);
    }

    #[test]
    fn test_not_warmup_when_agent() {
        // tool_result → agent → not warmup
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_123", "content": "ok"}
                ]}
            ]
        });
        let result = classify_request(&body, true, true);
        assert_eq!(result.initiator, "agent");
        assert!(!result.is_warmup);
    }

    // === merge_tool_results tests ===

    #[test]
    fn test_merge_intra_message_tool_result_text() {
        // Core scenario: tool_result + text in one message → text absorbed into tool_result
        let body = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "file contents"},
                    {"type": "text", "text": "skill output here"}
                ]}
            ]
        });
        let result = merge_tool_results(body);
        let content = result["messages"][0]["content"].as_array().unwrap();
        // Only 1 tool_result block should remain (text absorbed)
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_result");
        // The tool_result content should contain the original content plus the absorbed text
        let tr_content = content[0]["content"].as_str().unwrap();
        assert!(tr_content.contains("file contents"));
        assert!(tr_content.contains("skill output here"));
    }

    #[test]
    fn test_merge_intra_message_equal_count() {
        // Equal counts: paired one to one
        let body = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "result1"},
                    {"type": "text", "text": "text1"},
                    {"type": "tool_result", "tool_use_id": "t2", "content": "result2"},
                    {"type": "text", "text": "text2"}
                ]}
            ]
        });
        let result = merge_tool_results(body);
        let content = result["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert!(content[0]["content"].as_str().unwrap().contains("text1"));
        assert!(content[1]["content"].as_str().unwrap().contains("text2"));
    }

    #[test]
    fn test_merge_intra_message_empty_text_ignored() {
        // An empty text block appends nothing
        let body = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "result"},
                    {"type": "text", "text": ""}
                ]}
            ]
        });
        let result = merge_tool_results(body);
        let content = result["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        // Empty text leaves the original content unchanged
        assert_eq!(content[0]["content"], "result");
    }

    #[test]
    fn test_merge_intra_skips_other_block_types() {
        // A block that is neither tool_result nor text → skip the whole message
        let body = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "result"},
                    {"type": "image", "source": {"data": "..."}},
                    {"type": "text", "text": "caption"}
                ]}
            ]
        });
        let result = merge_tool_results(body);
        let content = result["messages"][0]["content"].as_array().unwrap();
        // Not merged; the 3 blocks stay as they were
        assert_eq!(content.len(), 3);
    }

    #[test]
    fn test_merge_cross_message_consecutive() {
        // Cross-message merge: consecutive tool_result-only user messages
        let body = json!({
            "messages": [
                {"role": "user", "content": "Read files"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "t2", "name": "Read", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "file1"}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t2", "content": "file2"}
                ]}
            ]
        });
        let result = merge_tool_results(body);
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        let merged_content = messages[2]["content"].as_array().unwrap();
        assert_eq!(merged_content.len(), 2);
    }

    #[test]
    fn test_merge_does_not_affect_normal_messages() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi!"},
                {"role": "user", "content": "How are you?"}
            ]
        });
        let result = merge_tool_results(body.clone());
        assert_eq!(result["messages"], body["messages"]);
    }

    // === deterministic_request_id tests ===

    #[test]
    fn test_deterministic_id_stable() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let id1 = deterministic_request_id(&body, "session1");
        let id2 = deterministic_request_id(&body, "session1");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_deterministic_id_varies_by_content() {
        let body1 = json!({
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let body2 = json!({
            "messages": [{"role": "user", "content": "Goodbye"}]
        });
        let id1 = deterministic_request_id(&body1, "session1");
        let id2 = deterministic_request_id(&body2, "session1");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_deterministic_id_varies_by_session() {
        let body = json!({
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let id1 = deterministic_request_id(&body, "session1");
        let id2 = deterministic_request_id(&body, "session2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_deterministic_id_ignores_tool_result() {
        // Different tool_result content but the same user text → same ID
        let body1 = json!({
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "version_A"}
                ]},
                {"role": "user", "content": "do something"}
            ]
        });
        let body2 = json!({
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "version_B"}
                ]},
                {"role": "user", "content": "do something"}
            ]
        });
        let id1 = deterministic_request_id(&body1, "s");
        let id2 = deterministic_request_id(&body2, "s");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_deterministic_id_fallback_when_no_user_content() {
        // No user message → falls back to a random UUID (different each time)
        let body = json!({
            "messages": [
                {"role": "assistant", "content": "Hi"}
            ]
        });
        let id1 = deterministic_request_id(&body, "s");
        let id2 = deterministic_request_id(&body, "s");
        // Random UUID, different each time
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_deterministic_id_is_valid_uuid() {
        let body = json!({
            "messages": [{"role": "user", "content": "test"}]
        });
        let id = deterministic_request_id(&body, "session");
        assert!(Uuid::parse_str(&id).is_ok());
    }

    // === compact detection tests ===

    #[test]
    fn test_compact_detection_system_prompt() {
        let body = json!({
            "system": "You are a helpful AI assistant tasked with summarizing conversations. Please provide a concise summary.",
            "messages": [
                {"role": "user", "content": "Here is the conversation to summarize..."}
            ]
        });
        assert!(is_compact_request(&body));
    }

    #[test]
    fn test_compact_detection_critical_keyword() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "CRITICAL: Respond with TEXT ONLY. Do NOT call any tools. Summarize this conversation."}
            ]
        });
        assert!(is_compact_request(&body));
    }

    #[test]
    fn test_compact_detection_structural_markers() {
        // Claude Code compact structure markers
        let body = json!({
            "messages": [
                {"role": "user", "content": "Summary of conversation:\n\nPending Tasks:\n- Fix bug\n\nCurrent Work:\n- Implementing feature"}
            ]
        });
        assert!(is_compact_request(&body));
    }

    #[test]
    fn test_compact_no_false_positive_on_generic_summary() {
        // A generic phrase must not trigger compact detection
        let body = json!({
            "messages": [
                {"role": "user", "content": "Your task is to create a detailed summary of the conversation so far."}
            ]
        });
        assert!(!is_compact_request(&body));
    }

    #[test]
    fn test_compact_detection_negative() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "What is the weather today?"}
            ]
        });
        assert!(!is_compact_request(&body));
    }

    #[test]
    fn test_compact_detection_system_array() {
        let body = json!({
            "system": [
                {"type": "text", "text": "You are a helpful AI assistant tasked with summarizing conversations."}
            ],
            "messages": [
                {"role": "user", "content": "Summarize"}
            ]
        });
        assert!(is_compact_request(&body));
    }
}
