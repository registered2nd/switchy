//! Proxy session: request session management
//!
//! Creates a session context for each proxy request and tracks state and metadata over its lifetime.
//!
//! ## Session ID extraction
//!
//! Extracts a session ID from the client request to link requests in the same conversation:
//! - Claude: from `metadata.user_id` (format: `user_xxx_session_yyy`) or `metadata.session_id`
//! - Codex: from `previous_response_id` or `session_id` in the headers
//! - Others: generate a new UUID

use axum::http::HeaderMap;
use std::time::Instant;
use uuid::Uuid;

/// Client request format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ClientFormat {
    /// Claude Messages API (/v1/messages)
    Claude,
    /// Codex Response API (/v1/responses)
    Codex,
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenAI,
    /// Gemini API (/v1beta/models/*/generateContent)
    Gemini,
    /// Gemini CLI API (/v1internal/models/*/generateContent)
    GeminiCli,
    /// Unknown format
    Unknown,
}

#[allow(dead_code)]
impl ClientFormat {
    /// Detect the format from the request path
    pub fn from_path(path: &str) -> Self {
        if path.contains("/v1/messages") {
            ClientFormat::Claude
        } else if path.contains("/v1/responses") {
            ClientFormat::Codex
        } else if path.contains("/v1/chat/completions") {
            ClientFormat::OpenAI
        } else if path.contains("/v1internal/") && path.contains("generateContent") {
            // Gemini CLI uses the /v1internal/ path
            ClientFormat::GeminiCli
        } else if (path.contains("/v1beta/") || path.contains("/v1/"))
            && path.contains("generateContent")
        {
            // Gemini API uses the /v1beta/ or /v1/ path
            ClientFormat::Gemini
        } else if path.contains("generateContent") {
            // Generic Gemini endpoint
            ClientFormat::Gemini
        } else {
            ClientFormat::Unknown
        }
    }

    /// Detect the format from the request body (fallback)
    pub fn from_body(body: &serde_json::Value) -> Self {
        // Claude format: messages array + model field + no response_format
        if body.get("messages").is_some()
            && body.get("model").is_some()
            && body.get("response_format").is_none()
            && body.get("contents").is_none()
        {
            // Tell Claude and OpenAI apart
            if body.get("max_tokens").is_some() {
                return ClientFormat::Claude;
            }
            return ClientFormat::OpenAI;
        }

        // Codex format: input field
        if body.get("input").is_some() {
            return ClientFormat::Codex;
        }

        // Gemini format: contents array
        if body.get("contents").is_some() {
            return ClientFormat::Gemini;
        }

        ClientFormat::Unknown
    }

    /// Convert to a string
    pub fn as_str(&self) -> &'static str {
        match self {
            ClientFormat::Claude => "claude",
            ClientFormat::Codex => "codex",
            ClientFormat::OpenAI => "openai",
            ClientFormat::Gemini => "gemini",
            ClientFormat::GeminiCli => "gemini_cli",
            ClientFormat::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ClientFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Proxy session
///
/// Context data for the whole request lifetime
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProxySession {
    /// Unique session ID
    pub session_id: String,
    /// Request start time
    pub start_time: Instant,
    /// HTTP method
    pub method: String,
    /// Request URL
    pub request_url: String,
    /// User-Agent
    pub user_agent: Option<String>,
    /// Client request format
    pub client_format: ClientFormat,
    /// Selected provider ID
    pub provider_id: Option<String>,
    /// Model name
    pub model: Option<String>,
    /// Whether the request is streaming
    pub is_streaming: bool,
}

#[allow(dead_code)]
impl ProxySession {
    /// Create a session from a request
    pub fn from_request(
        method: &str,
        request_url: &str,
        user_agent: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> Self {
        // Detect the client format
        let mut client_format = ClientFormat::from_path(request_url);
        if client_format == ClientFormat::Unknown {
            if let Some(body) = body {
                client_format = ClientFormat::from_body(body);
            }
        }

        // Detect whether the request is streaming
        let is_streaming = body
            .and_then(|b| b.get("stream"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Extract the model name
        let model = body
            .and_then(|b| b.get("model"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Self {
            session_id: Uuid::new_v4().to_string(),
            start_time: Instant::now(),
            method: method.to_string(),
            request_url: request_url.to_string(),
            user_agent: user_agent.map(|s| s.to_string()),
            client_format,
            provider_id: None,
            model,
            is_streaming,
        }
    }

    /// Set the provider ID
    pub fn with_provider(mut self, provider_id: &str) -> Self {
        self.provider_id = Some(provider_id.to_string());
        self
    }

    /// Get the request latency (ms)
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

// ============================================================================
// Session ID extractor
// ============================================================================

/// Session ID source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdSource {
    /// From metadata.user_id (Claude)
    MetadataUserId,
    /// From metadata.session_id
    MetadataSessionId,
    /// From headers (Codex)
    Header,
    /// From previous_response_id (Codex)
    PreviousResponseId,
    /// Newly generated
    Generated,
}

/// Session ID extraction result
#[derive(Debug, Clone)]
pub struct SessionIdResult {
    /// Extracted or generated session ID
    pub session_id: String,
    /// Session ID source
    pub source: SessionIdSource,
    /// Whether the ID came from the client (not newly generated)
    pub client_provided: bool,
}

/// Extract or generate a session ID from the request
///
/// Lightweight: extracts session_id for logging only, with no full session management.
///
/// ## Extraction priority
///
/// ### Claude requests
/// 1. `metadata.user_id` (format: `user_xxx_session_yyy`) → take the `yyy` part
/// 2. `metadata.session_id` → use as is
/// 3. Generate a new UUID
///
/// ### Codex requests
/// 1. Headers: `session_id` or `x-session-id`
/// 2. `metadata.session_id`
/// 3. `previous_response_id` (conversation continuation)
/// 4. Generate a new UUID
///
/// ## Example
///
/// ```ignore
/// let result = extract_session_id(&headers, &body, "claude");
/// println!("Session ID: {} (from {:?})", result.session_id, result.source);
/// ```
pub fn extract_session_id(
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_format: &str,
) -> SessionIdResult {
    // Codex requests get special handling
    if client_format == "codex" || client_format == "openai" {
        if let Some(result) = extract_codex_session(headers, body) {
            return result;
        }
    }

    // Claude requests: extract from metadata
    if let Some(result) = extract_from_metadata(body) {
        return result;
    }

    // Fallback: generate a new session ID
    generate_new_session_id()
}

/// Extract the Codex session ID
fn extract_codex_session(headers: &HeaderMap, body: &serde_json::Value) -> Option<SessionIdResult> {
    // 1. From headers
    for header_name in &["session_id", "x-session-id"] {
        if let Some(value) = headers.get(*header_name) {
            if let Ok(session_id) = value.to_str() {
                // Codex session IDs are usually long (UUID format)
                if session_id.len() > 20 {
                    return Some(SessionIdResult {
                        session_id: format!("codex_{session_id}"),
                        source: SessionIdSource::Header,
                        client_provided: true,
                    });
                }
            }
        }
    }

    // 2. From body.metadata.session_id
    if let Some(session_id) = body
        .get("metadata")
        .and_then(|m| m.get("session_id"))
        .and_then(|v| v.as_str())
    {
        if session_id.len() > 10 {
            return Some(SessionIdResult {
                session_id: format!("codex_{session_id}"),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    // 3. From previous_response_id (conversation continuation)
    if let Some(prev_id) = body.get("previous_response_id").and_then(|v| v.as_str()) {
        if prev_id.len() > 10 {
            return Some(SessionIdResult {
                session_id: format!("codex_{prev_id}"),
                source: SessionIdSource::PreviousResponseId,
                client_provided: true,
            });
        }
    }

    None
}

/// Extract the session ID from metadata (Claude)
fn extract_from_metadata(body: &serde_json::Value) -> Option<SessionIdResult> {
    let metadata = body.get("metadata")?;

    // 1. From metadata.user_id (format: user_xxx_session_yyy)
    if let Some(user_id) = metadata.get("user_id").and_then(|v| v.as_str()) {
        if let Some(session_id) = parse_session_from_user_id(user_id) {
            return Some(SessionIdResult {
                session_id,
                source: SessionIdSource::MetadataUserId,
                client_provided: true,
            });
        }
    }

    // 2. Directly from metadata.session_id
    if let Some(session_id) = metadata.get("session_id").and_then(|v| v.as_str()) {
        if !session_id.is_empty() {
            return Some(SessionIdResult {
                session_id: session_id.to_string(),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    None
}

/// Parse session_id from user_id
///
/// Format: `user_identifier_session_actual_session_id`
fn parse_session_from_user_id(user_id: &str) -> Option<String> {
    // Find the "_session_" separator
    if let Some(pos) = user_id.find("_session_") {
        let session_id = &user_id[pos + 9..]; // "_session_" is 9 chars long
        if !session_id.is_empty() {
            return Some(session_id.to_string());
        }
    }
    None
}

/// Generate a new session ID
fn generate_new_session_id() -> SessionIdResult {
    SessionIdResult {
        session_id: Uuid::new_v4().to_string(),
        source: SessionIdSource::Generated,
        client_provided: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_client_format_from_path_claude() {
        assert_eq!(
            ClientFormat::from_path("/v1/messages"),
            ClientFormat::Claude
        );
        assert_eq!(
            ClientFormat::from_path("/api/v1/messages"),
            ClientFormat::Claude
        );
    }

    #[test]
    fn test_client_format_from_path_codex() {
        assert_eq!(
            ClientFormat::from_path("/v1/responses"),
            ClientFormat::Codex
        );
    }

    #[test]
    fn test_client_format_from_path_openai() {
        assert_eq!(
            ClientFormat::from_path("/v1/chat/completions"),
            ClientFormat::OpenAI
        );
    }

    #[test]
    fn test_client_format_from_path_gemini() {
        assert_eq!(
            ClientFormat::from_path("/v1beta/models/gemini-pro:generateContent"),
            ClientFormat::Gemini
        );
    }

    #[test]
    fn test_client_format_from_path_gemini_cli() {
        assert_eq!(
            ClientFormat::from_path("/v1internal/models/gemini-pro:generateContent"),
            ClientFormat::GeminiCli
        );
    }

    #[test]
    fn test_client_format_from_body_claude() {
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 1024
        });
        assert_eq!(ClientFormat::from_body(&body), ClientFormat::Claude);
    }

    #[test]
    fn test_client_format_from_body_codex() {
        let body = json!({
            "input": "Write a function"
        });
        assert_eq!(ClientFormat::from_body(&body), ClientFormat::Codex);
    }

    #[test]
    fn test_client_format_from_body_gemini() {
        let body = json!({
            "contents": [{"parts": [{"text": "Hello"}]}]
        });
        assert_eq!(ClientFormat::from_body(&body), ClientFormat::Gemini);
    }

    #[test]
    fn test_session_id_uniqueness() {
        let session1 = ProxySession::from_request("POST", "/v1/messages", None, None);
        let session2 = ProxySession::from_request("POST", "/v1/messages", None, None);
        assert_ne!(session1.session_id, session2.session_id);
    }

    #[test]
    fn test_session_from_request() {
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 1024,
            "stream": true
        });

        let session =
            ProxySession::from_request("POST", "/v1/messages", Some("Mozilla/5.0"), Some(&body));

        assert_eq!(session.method, "POST");
        assert_eq!(session.request_url, "/v1/messages");
        assert_eq!(session.user_agent, Some("Mozilla/5.0".to_string()));
        assert_eq!(session.client_format, ClientFormat::Claude);
        assert_eq!(session.model, Some("claude-3-5-sonnet".to_string()));
        assert!(session.is_streaming);
    }

    #[test]
    fn test_session_with_provider() {
        let session = ProxySession::from_request("POST", "/v1/messages", None, None)
            .with_provider("provider-123");

        assert_eq!(session.provider_id, Some("provider-123".to_string()));
    }

    #[test]
    fn test_client_format_as_str() {
        assert_eq!(ClientFormat::Claude.as_str(), "claude");
        assert_eq!(ClientFormat::Codex.as_str(), "codex");
        assert_eq!(ClientFormat::OpenAI.as_str(), "openai");
        assert_eq!(ClientFormat::Gemini.as_str(), "gemini");
        assert_eq!(ClientFormat::GeminiCli.as_str(), "gemini_cli");
        assert_eq!(ClientFormat::Unknown.as_str(), "unknown");
    }

    // ========== Session ID extraction tests ==========

    #[test]
    fn test_extract_session_from_claude_metadata_user_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {
                "user_id": "user_john_doe_session_abc123def456"
            }
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "abc123def456");
        assert_eq!(result.source, SessionIdSource::MetadataUserId);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_from_claude_metadata_session_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {
                "session_id": "my-session-123"
            }
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert_eq!(result.session_id, "my-session-123");
        assert_eq!(result.source, SessionIdSource::MetadataSessionId);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_from_codex_previous_response_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "input": "Write a function",
            "previous_response_id": "resp_abc123def456789"
        });

        let result = extract_session_id(&headers, &body, "codex");

        assert_eq!(result.session_id, "codex_resp_abc123def456789");
        assert_eq!(result.source, SessionIdSource::PreviousResponseId);
        assert!(result.client_provided);
    }

    #[test]
    fn test_extract_session_generates_new_when_not_found() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = extract_session_id(&headers, &body, "claude");

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_parse_session_from_user_id() {
        assert_eq!(
            parse_session_from_user_id("user_john_session_abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_session_from_user_id("my_app_session_xyz789"),
            Some("xyz789".to_string())
        );
        // "_session_" is the separator, so the string below matches
        assert_eq!(
            parse_session_from_user_id("no_session_marker"),
            Some("marker".to_string())
        );
        // No "_session_" separator
        assert_eq!(parse_session_from_user_id("user_john_abc123"), None);
        assert_eq!(parse_session_from_user_id("_session_"), None);
    }
}
