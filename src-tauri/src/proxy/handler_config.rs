//! Handler configuration
//!
//! Configuration structs and usage parsers for each API handler

use crate::app_config::AppType;
use crate::proxy::usage::parser::TokenUsage;
use serde_json::Value;

/// Usage parser type aliases
pub type StreamUsageParser = fn(&[Value]) -> Option<TokenUsage>;
pub type ResponseUsageParser = fn(&Value) -> Option<TokenUsage>;

/// Model extractor type alias
/// Arguments: (stream events, model name in the request) -> model name actually used
pub type StreamModelExtractor = fn(&[Value], &str) -> String;

/// Usage parsing configuration per API
#[derive(Clone, Copy)]
pub struct UsageParserConfig {
    /// Streaming response parser
    pub stream_parser: StreamUsageParser,
    /// Non-streaming response parser
    pub response_parser: ResponseUsageParser,
    /// Model extractor for streaming responses
    pub model_extractor: StreamModelExtractor,
    /// App type string (for logging)
    pub app_type_str: &'static str,
}

// ============================================================================
// Model extractors
// ============================================================================

/// Claude streaming model extraction (prefers usage.model)
fn claude_model_extractor(events: &[Value], request_model: &str) -> String {
    // First try the model from the parsed usage
    if let Some(usage) = TokenUsage::from_claude_stream_events(events) {
        if let Some(model) = usage.model {
            return model;
        }
    }
    request_model.to_string()
}

/// OpenAI Chat Completions streaming model extraction (prefers usage.model)
fn openai_model_extractor(events: &[Value], request_model: &str) -> String {
    // First try the model from the parsed usage
    if let Some(usage) = TokenUsage::from_openai_stream_events(events) {
        if let Some(model) = usage.model {
            return model;
        }
    }
    // Fallback: extract directly from the events
    events
        .iter()
        .find_map(|e| e.get("model")?.as_str())
        .unwrap_or(request_model)
        .to_string()
}

/// Codex streaming model extraction (detects the format)
fn codex_auto_model_extractor(events: &[Value], request_model: &str) -> String {
    // First try the model from the parsed usage
    if let Some(usage) = TokenUsage::from_codex_stream_events_auto(events) {
        if let Some(model) = usage.model {
            return model;
        }
    }
    // Fallback: extract from the response.completed event
    events
        .iter()
        .find_map(|e| {
            if e.get("type")?.as_str()? == "response.completed" {
                e.get("response")?.get("model")?.as_str()
            } else {
                None
            }
        })
        .or_else(|| {
            // Last fallback: extract from OpenAI-format events
            events.iter().find_map(|e| e.get("model")?.as_str())
        })
        .unwrap_or(request_model)
        .to_string()
}

/// Gemini streaming model extraction (prefers usage.model)
fn gemini_model_extractor(events: &[Value], request_model: &str) -> String {
    // First try the model from the parsed usage
    if let Some(usage) = TokenUsage::from_gemini_stream_chunks(events) {
        if let Some(model) = usage.model {
            return model;
        }
    }
    request_model.to_string()
}

// ============================================================================
// Predefined configurations
// ============================================================================

/// Claude API parsing configuration
pub const CLAUDE_PARSER_CONFIG: UsageParserConfig = UsageParserConfig {
    stream_parser: TokenUsage::from_claude_stream_events,
    response_parser: TokenUsage::from_claude_response,
    model_extractor: claude_model_extractor,
    app_type_str: "claude",
};

/// OpenAI Chat Completions API parsing configuration (for Codex /v1/chat/completions)
pub const OPENAI_PARSER_CONFIG: UsageParserConfig = UsageParserConfig {
    stream_parser: TokenUsage::from_openai_stream_events,
    response_parser: TokenUsage::from_openai_response,
    model_extractor: openai_model_extractor,
    app_type_str: "codex",
};

/// Codex parsing configuration (detects OpenAI or Codex format)
pub const CODEX_PARSER_CONFIG: UsageParserConfig = UsageParserConfig {
    stream_parser: TokenUsage::from_codex_stream_events_auto,
    response_parser: TokenUsage::from_codex_response_auto,
    model_extractor: codex_auto_model_extractor,
    app_type_str: "codex",
};

/// Gemini API parsing configuration
pub const GEMINI_PARSER_CONFIG: UsageParserConfig = UsageParserConfig {
    stream_parser: TokenUsage::from_gemini_stream_chunks,
    response_parser: TokenUsage::from_gemini_response,
    model_extractor: gemini_model_extractor,
    app_type_str: "gemini",
};

// ============================================================================
// Handler configuration (reserved for further simplification)
// ============================================================================

/// Base handler configuration
///
/// Reserved; could unify the configuration of all handlers
#[allow(dead_code)]
#[derive(Clone)]
pub struct HandlerConfig {
    /// App type
    pub app_type: AppType,
    /// Log tag
    pub tag: &'static str,
    /// App type string
    pub app_type_str: &'static str,
    /// Usage parsing configuration
    pub parser_config: &'static UsageParserConfig,
}

/// Claude handler configuration
#[allow(dead_code)]
pub const CLAUDE_HANDLER_CONFIG: HandlerConfig = HandlerConfig {
    app_type: AppType::Claude,
    tag: "Claude",
    app_type_str: "claude",
    parser_config: &CLAUDE_PARSER_CONFIG,
};

/// Codex Chat Completions handler configuration
#[allow(dead_code)]
pub const CODEX_CHAT_HANDLER_CONFIG: HandlerConfig = HandlerConfig {
    app_type: AppType::Codex,
    tag: "Codex",
    app_type_str: "codex",
    parser_config: &OPENAI_PARSER_CONFIG,
};

/// Codex Responses handler configuration
#[allow(dead_code)]
pub const CODEX_RESPONSES_HANDLER_CONFIG: HandlerConfig = HandlerConfig {
    app_type: AppType::Codex,
    tag: "Codex",
    app_type_str: "codex",
    parser_config: &CODEX_PARSER_CONFIG,
};

/// Gemini handler configuration
#[allow(dead_code)]
pub const GEMINI_HANDLER_CONFIG: HandlerConfig = HandlerConfig {
    app_type: AppType::Gemini,
    tag: "Gemini",
    app_type_str: "gemini",
    parser_config: &GEMINI_PARSER_CONFIG,
};
