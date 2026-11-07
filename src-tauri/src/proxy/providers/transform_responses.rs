//! OpenAI Responses API format conversion
//!
//! Converts between Anthropic Messages and the OpenAI Responses API.
//! The Responses API is OpenAI's newer API, released in 2025, with a flat input/output structure.
//!
//! Main differences from Chat Completions:
//! - tool_use/tool_result are "lifted" out of message content into top-level input items
//! - the system prompt uses the `instructions` field instead of a system-role message
//! - usage field names match Anthropic's (input_tokens/output_tokens)

use crate::proxy::error::ProxyError;
use serde_json::{json, Value};

/// Anthropic request → OpenAI Responses request
///
/// `cache_key`: optional prompt_cache_key to inject for improved cache routing
pub fn anthropic_to_responses(body: Value, cache_key: Option<&str>) -> Result<Value, ProxyError> {
    let mut result = json!({});

    // NOTE: model mapping happens upstream (proxy::model_mapper); this layer only converts structure.
    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        result["model"] = json!(model);
    }

    // system → instructions (the Responses API uses the instructions field)
    if let Some(system) = body.get("system") {
        let instructions = if let Some(text) = system.as_str() {
            text.to_string()
        } else if let Some(arr) = system.as_array() {
            arr.iter()
                .filter_map(|msg| msg.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            String::new()
        };
        if !instructions.is_empty() {
            result["instructions"] = json!(instructions);
        }
    }

    // messages → input
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        let input = convert_messages_to_input(msgs)?;
        result["input"] = json!(input);
    }

    // max_tokens → max_output_tokens (Responses API uses max_output_tokens for all models)
    if let Some(v) = body.get("max_tokens") {
        result["max_output_tokens"] = v.clone();
    }

    // Parameters passed through as-is
    if let Some(v) = body.get("temperature") {
        result["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        result["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stream") {
        result["stream"] = v.clone();
    }

    // Map Anthropic thinking → OpenAI Responses reasoning.effort
    if let Some(model_name) = body.get("model").and_then(|m| m.as_str()) {
        if super::transform::supports_reasoning_effort(model_name) {
            if let Some(effort) = super::transform::resolve_reasoning_effort(&body) {
                result["reasoning"] = json!({ "effort": effort });
            }
        }
    }

    // stop_sequences → dropped (the Responses API does not support them)

    // Convert tools (BatchTool filtered out)
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let response_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) != Some("BatchTool"))
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                    "description": t.get("description"),
                    "parameters": super::transform::clean_schema(
                        t.get("input_schema").cloned().unwrap_or(json!({}))
                    )
                })
            })
            .collect();

        if !response_tools.is_empty() {
            result["tools"] = json!(response_tools);
        }
    }

    if let Some(v) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_responses(v);
    }

    // Inject prompt_cache_key for improved cache routing on OpenAI-compatible endpoints
    if let Some(key) = cache_key {
        result["prompt_cache_key"] = json!(key);
    }

    Ok(result)
}

fn map_tool_choice_to_responses(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(_) => tool_choice.clone(),
        Value::Object(obj) => match obj.get("type").and_then(|t| t.as_str()) {
            // Anthropic "any" means at least one tool call is required
            Some("any") => json!("required"),
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            // Anthropic forced tool -> Responses function tool selector
            Some("tool") => {
                let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                json!({
                    "type": "function",
                    "name": name
                })
            }
            _ => tool_choice.clone(),
        },
        _ => tool_choice.clone(),
    }
}

pub(crate) fn map_responses_stop_reason(
    status: Option<&str>,
    has_tool_use: bool,
    incomplete_reason: Option<&str>,
) -> Option<&'static str> {
    status.map(|s| match s {
        "completed" => {
            if has_tool_use {
                "tool_use"
            } else {
                "end_turn"
            }
        }
        "incomplete" => {
            if matches!(
                incomplete_reason,
                Some("max_output_tokens") | Some("max_tokens")
            ) || incomplete_reason.is_none()
            {
                "max_tokens"
            } else {
                "end_turn"
            }
        }
        _ => "end_turn",
    })
}

/// Build Anthropic-style usage JSON from Responses API usage, including cache tokens.
///
/// Priority order:
/// 1. OpenAI nested details (`input_tokens_details.cached_tokens`, `prompt_tokens_details.cached_tokens`) as initial value
/// 2. Direct Anthropic-style fields (`cache_read_input_tokens`, `cache_creation_input_tokens`) override if present
pub(crate) fn build_anthropic_usage_from_responses(usage: Option<&Value>) -> Value {
    let u = match usage {
        Some(v) if !v.is_null() => v,
        _ => {
            return json!({
                "input_tokens": 0,
                "output_tokens": 0
            })
        }
    };

    let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut result = json!({
        "input_tokens": input,
        "output_tokens": output
    });

    // Step 1: OpenAI nested details as fallback
    // OpenAI Responses API: input_tokens_details.cached_tokens
    if let Some(cached) = u
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
    {
        result["cache_read_input_tokens"] = json!(cached);
    }
    // OpenAI standard: prompt_tokens_details.cached_tokens
    if let Some(cached) = u
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
    {
        if result.get("cache_read_input_tokens").is_none() {
            result["cache_read_input_tokens"] = json!(cached);
        }
    }

    // Step 2: Direct Anthropic-style fields override (authoritative if present)
    if let Some(v) = u.get("cache_read_input_tokens") {
        result["cache_read_input_tokens"] = v.clone();
    }
    if let Some(v) = u.get("cache_creation_input_tokens") {
        result["cache_creation_input_tokens"] = v.clone();
    }

    result
}

/// Converts the Anthropic messages array to a Responses API input array
///
/// Core conversion:
/// - user/assistant text content → a message item with that role
/// - tool_use is "lifted" out of the assistant message into its own function_call item
/// - tool_result is "lifted" out of the user message into its own function_call_output item
/// - thinking blocks → dropped
fn convert_messages_to_input(messages: &[Value]) -> Result<Vec<Value>, ProxyError> {
    let mut input = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content");

        match content {
            // String content
            Some(Value::String(text)) => {
                let content_type = if role == "assistant" {
                    "output_text"
                } else {
                    "input_text"
                };
                input.push(json!({
                    "role": role,
                    "content": [{ "type": content_type, "text": text }]
                }));
            }

            // Array content (multimodal / tool calls)
            Some(Value::Array(blocks)) => {
                let mut message_content = Vec::new();

                for block in blocks {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    match block_type {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                let content_type = if role == "assistant" {
                                    "output_text"
                                } else {
                                    "input_text"
                                };
                                // OpenAI Responses API does not accept Anthropic cache_control
                                // under input[].content[].
                                message_content.push(json!({ "type": content_type, "text": text }));
                            }
                        }

                        "image" => {
                            if let Some(source) = block.get("source") {
                                let media_type = source
                                    .get("media_type")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("image/png");
                                let data =
                                    source.get("data").and_then(|d| d.as_str()).unwrap_or("");
                                message_content.push(json!({
                                    "type": "input_image",
                                    "image_url": format!("data:{media_type};base64,{data}")
                                }));
                            }
                        }

                        "tool_use" => {
                            // Flush the accumulated message content first
                            if !message_content.is_empty() {
                                input.push(json!({
                                    "role": role,
                                    "content": message_content.clone()
                                }));
                                message_content.clear();
                            }

                            // Lift into its own function_call item
                            let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let arguments = block.get("input").cloned().unwrap_or(json!({}));

                            input.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": serde_json::to_string(&arguments).unwrap_or_default()
                            }));
                        }

                        "tool_result" => {
                            // Flush the accumulated message content first
                            if !message_content.is_empty() {
                                input.push(json!({
                                    "role": role,
                                    "content": message_content.clone()
                                }));
                                message_content.clear();
                            }

                            // Lift into its own function_call_output item
                            let call_id = block
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let output = match block.get("content") {
                                Some(Value::String(s)) => s.clone(),
                                Some(v) => serde_json::to_string(v).unwrap_or_default(),
                                None => String::new(),
                            };

                            input.push(json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": output
                            }));
                        }

                        "thinking" => {
                            // Drop thinking blocks (same as openai_chat)
                        }

                        _ => {}
                    }
                }

                // Flush the remaining message content
                if !message_content.is_empty() {
                    input.push(json!({
                        "role": role,
                        "content": message_content
                    }));
                }
            }

            _ => {
                // No content, or null
                input.push(json!({ "role": role }));
            }
        }
    }

    Ok(input)
}

/// OpenAI Responses response → Anthropic response
pub fn responses_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let output = body
        .get("output")
        .and_then(|o| o.as_array())
        .ok_or_else(|| ProxyError::TransformError("No output in response".to_string()))?;

    let mut content = Vec::new();

    let mut has_tool_use = false;
    for item in output {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match item_type {
            "message" => {
                if let Some(msg_content) = item.get("content").and_then(|c| c.as_array()) {
                    for block in msg_content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if block_type == "output_text" {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    content.push(json!({"type": "text", "text": text}));
                                }
                            }
                        } else if block_type == "refusal" {
                            if let Some(refusal) = block.get("refusal").and_then(|t| t.as_str()) {
                                if !refusal.is_empty() {
                                    content.push(json!({"type": "text", "text": refusal}));
                                }
                            }
                        }
                    }
                }
            }

            "function_call" => {
                let call_id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args_str = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                content.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                }));
                has_tool_use = true;
            }

            "reasoning" => {
                // Map reasoning summary → thinking block
                if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                    let thinking_text: String = summary
                        .iter()
                        .filter_map(|s| {
                            if s.get("type").and_then(|t| t.as_str()) == Some("summary_text") {
                                s.get("text").and_then(|t| t.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    if !thinking_text.is_empty() {
                        content.push(json!({
                            "type": "thinking",
                            "thinking": thinking_text
                        }));
                    }
                }
            }

            _ => {}
        }
    }

    // status → stop_reason
    let stop_reason = map_responses_stop_reason(
        body.get("status").and_then(|s| s.as_str()),
        has_tool_use,
        body.pointer("/incomplete_details/reason")
            .and_then(|r| r.as_str()),
    );

    let usage_json = build_anthropic_usage_from_responses(body.get("usage"));

    let result = json!({
        "id": body.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage_json
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_to_responses_simple() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["model"], "gpt-4o");
        assert_eq!(result["max_output_tokens"], 1024);
        assert_eq!(result["input"][0]["role"], "user");
        assert_eq!(result["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(result["input"][0]["content"][0]["text"], "Hello");
        // stop_sequences should not appear
        assert!(result.get("stop_sequences").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_with_system_string() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "system": "You are a helpful assistant.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["instructions"], "You are a helpful assistant.");
        // system should not appear in input
        assert_eq!(result["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_anthropic_to_responses_with_system_array() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "Part 1"},
                {"type": "text", "text": "Part 2"}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["instructions"], "Part 1\n\nPart 2");
    }

    #[test]
    fn test_anthropic_to_responses_with_tools() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather info",
                "input_schema": {"type": "object", "properties": {"location": {"type": "string"}}}
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["tools"][0]["type"], "function");
        assert_eq!(result["tools"][0]["name"], "get_weather");
        assert!(result["tools"][0].get("parameters").is_some());
        // input_schema should not appear
        assert!(result["tools"][0].get("input_schema").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_tool_choice_any_to_required() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tool_choice": {"type": "any"}
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["tool_choice"], "required");
    }

    #[test]
    fn test_anthropic_to_responses_tool_choice_tool_to_function() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["tool_choice"]["type"], "function");
        assert_eq!(result["tool_choice"]["name"], "get_weather");
    }

    #[test]
    fn test_anthropic_to_responses_tool_use_lifting() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Let me check"},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // Should produce: assistant message (text) + function_call item
        assert_eq!(input_arr.len(), 2);

        // First: assistant message with output_text
        assert_eq!(input_arr[0]["role"], "assistant");
        assert_eq!(input_arr[0]["content"][0]["type"], "output_text");
        assert_eq!(input_arr[0]["content"][0]["text"], "Let me check");

        // Second: function_call item (lifted from message)
        assert_eq!(input_arr[1]["type"], "function_call");
        assert_eq!(input_arr[1]["call_id"], "call_123");
        assert_eq!(input_arr[1]["name"], "get_weather");
    }

    #[test]
    fn test_anthropic_to_responses_tool_result_lifting() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_123", "content": "Sunny, 25°C"}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // Should produce: function_call_output item (lifted)
        assert_eq!(input_arr.len(), 1);
        assert_eq!(input_arr[0]["type"], "function_call_output");
        assert_eq!(input_arr[0]["call_id"], "call_123");
        assert_eq!(input_arr[0]["output"], "Sunny, 25°C");
    }

    #[test]
    fn test_anthropic_to_responses_thinking_discarded() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Let me think..."},
                    {"type": "text", "text": "The answer is 42"}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // thinking should be discarded, only text remains
        assert_eq!(input_arr.len(), 1);
        assert_eq!(input_arr[0]["content"][0]["type"], "output_text");
        assert_eq!(input_arr[0]["content"][0]["text"], "The answer is 42");
    }

    #[test]
    fn test_anthropic_to_responses_image() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is this?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc123"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        let content = result["input"][0]["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,abc123");
    }

    #[test]
    fn test_responses_to_anthropic_simple() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "id": "msg_123",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["id"], "resp_123");
        assert_eq!(result["type"], "message");
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "Hello!");
        assert_eq!(result["stop_reason"], "end_turn");
        assert_eq!(result["usage"]["input_tokens"], 10);
        assert_eq!(result["usage"]["output_tokens"], 5);
    }

    #[test]
    fn test_responses_to_anthropic_with_function_call() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "function_call",
                "id": "fc_123",
                "call_id": "call_123",
                "name": "get_weather",
                "arguments": "{\"location\": \"Tokyo\"}",
                "status": "completed"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 15}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "tool_use");
        assert_eq!(result["content"][0]["id"], "call_123");
        assert_eq!(result["content"][0]["name"], "get_weather");
        assert_eq!(result["content"][0]["input"]["location"], "Tokyo");
        assert_eq!(result["stop_reason"], "tool_use");
    }

    #[test]
    fn test_responses_to_anthropic_with_refusal_block() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "refusal", "refusal": "I can't help with that."}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "I can't help with that.");
        assert_eq!(result["stop_reason"], "end_turn");
    }

    #[test]
    fn test_responses_to_anthropic_with_reasoning() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_123",
                    "summary": [
                        {"type": "summary_text", "text": "Thinking about the problem..."}
                    ]
                },
                {
                    "type": "message",
                    "id": "msg_123",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "The answer is 42"}]
                }
            ],
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });

        let result = responses_to_anthropic(input).unwrap();
        // Should have thinking + text
        assert_eq!(result["content"][0]["type"], "thinking");
        assert_eq!(
            result["content"][0]["thinking"],
            "Thinking about the problem..."
        );
        assert_eq!(result["content"][1]["type"], "text");
        assert_eq!(result["content"][1]["text"], "The answer is 42");
    }

    #[test]
    fn test_responses_to_anthropic_incomplete_status() {
        let input = json!({
            "id": "resp_123",
            "status": "incomplete",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Partial..."}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 4096}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["stop_reason"], "max_tokens");
    }

    #[test]
    fn test_responses_to_anthropic_incomplete_non_token_reason() {
        let input = json!({
            "id": "resp_123",
            "status": "incomplete",
            "incomplete_details": {"reason": "content_filter"},
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Blocked"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 1}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["stop_reason"], "end_turn");
    }

    #[test]
    fn test_model_passthrough() {
        let input = json!({
            "model": "o3-mini",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["model"], "o3-mini");
    }

    #[test]
    fn test_anthropic_to_responses_with_cache_key() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, Some("my-provider-id")).unwrap();
        assert_eq!(result["prompt_cache_key"], "my-provider-id");
    }

    #[test]
    fn test_anthropic_to_responses_strip_cache_control_on_tools() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object"},
                "cache_control": {"type": "ephemeral"}
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert!(result["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_strip_cache_control_on_text() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Hello", "cache_control": {"type": "ephemeral"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert!(result["input"][0]["content"][0]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn test_responses_to_anthropic_with_cache_tokens() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "input_tokens_details": {
                    "cached_tokens": 80
                }
            }
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["usage"]["input_tokens"], 100);
        assert_eq!(result["usage"]["output_tokens"], 50);
        assert_eq!(result["usage"]["cache_read_input_tokens"], 80);
    }

    #[test]
    fn test_responses_to_anthropic_with_direct_cache_fields() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 60,
                "cache_creation_input_tokens": 20
            }
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["usage"]["cache_read_input_tokens"], 60);
        assert_eq!(result["usage"]["cache_creation_input_tokens"], 20);
    }

    #[test]
    fn test_anthropic_to_responses_o_series_uses_max_output_tokens() {
        // Responses API always uses max_output_tokens, even for o-series models
        let input = json!({
            "model": "o3-mini",
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["max_output_tokens"], 4096);
        assert!(result.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_responses_output_config_max_sets_reasoning_xhigh() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "max"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn test_responses_output_config_takes_priority_over_thinking() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "low"},
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_responses_thinking_enabled_small_budget_sets_reasoning_low() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_responses_thinking_enabled_medium_budget_sets_reasoning_medium() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 8000},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "medium");
    }

    #[test]
    fn test_responses_thinking_enabled_large_budget_sets_reasoning_high() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 32000},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_responses_thinking_adaptive_sets_reasoning_high() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert_eq!(result["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_responses_non_reasoning_model_no_reasoning() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None).unwrap();
        assert!(result.get("reasoning").is_none());
    }
}
