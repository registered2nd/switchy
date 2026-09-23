//! Request handlers
//!
//! Handles HTTP requests for each API endpoint
//!
//! Structure:
//! - Shared logic lives in the `handler_context` and `response_processor` modules
//! - Each handler keeps only its own specific logic
//! - Claude's format-conversion logic stays in this file (fallback for the legacy OpenRouter endpoint)

use super::{
    error_mapper::{get_error_message, map_proxy_error_to_status},
    handler_config::{
        CLAUDE_PARSER_CONFIG, CODEX_PARSER_CONFIG, GEMINI_PARSER_CONFIG, OPENAI_PARSER_CONFIG,
    },
    handler_context::RequestContext,
    providers::{
        get_adapter, get_claude_api_format, streaming::create_anthropic_sse_stream,
        streaming_responses::create_anthropic_sse_stream_from_responses, transform,
        transform_responses,
    },
    response_processor::{
        create_logged_passthrough_stream, process_response, read_decoded_body,
        strip_entity_headers_for_rebuilt_body, SseUsageCollector,
    },
    server::ProxyState,
    types::*,
    usage::parser::TokenUsage,
    ProxyError,
};
use crate::app_config::AppType;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{json, Value};

// ============================================================================
// Health check and status (simple endpoints)
// ============================================================================

/// Health check
pub async fn health_check() -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "status": "healthy",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })),
    )
}

/// Service status
pub async fn get_status(State(state): State<ProxyState>) -> Result<Json<ProxyStatus>, ProxyError> {
    let status = state.status.read().await.clone();
    Ok(Json(status))
}

// ============================================================================
// Claude API handler (includes format conversion)
// ============================================================================

/// Handles /v1/messages requests (Claude API)
///
/// The Claude handler has its own format-conversion logic:
/// - previously used for OpenRouter's OpenAI Chat Completions-compatible endpoint (Anthropic <-> OpenAI conversion)
/// - OpenRouter now offers a Claude Code-compatible endpoint, so the conversion is off by default (the logic is kept as a fallback)
pub async fn handle_messages(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let uri = parts.uri;
    let headers = parts.headers;
    let extensions = parts.extensions;
    let body_bytes = body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ProxyError::Internal(format!("Failed to parse request body: {e}")))?;

    let mut ctx =
        RequestContext::new(&state, &body, &headers, AppType::Claude, "Claude", "claude").await?;

    let endpoint = uri
        .path_and_query()
        .map(|path_and_query| path_and_query.as_str())
        .unwrap_or(uri.path());

    let is_stream = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    // Forward the request
    let forwarder = ctx.create_forwarder(&state);
    let result = match forwarder
        .forward_with_retry(
            &AppType::Claude,
            endpoint,
            body.clone(),
            headers,
            extensions,
            ctx.get_providers(),
        )
        .await
    {
        Ok(result) => result,
        Err(mut err) => {
            if let Some(provider) = err.provider.take() {
                ctx.provider = provider;
            }
            log_forward_error(&state, &ctx, is_stream, &err.error);
            return Err(err.error);
        }
    };

    ctx.provider = result.provider;
    let api_format = result
        .claude_api_format
        .as_deref()
        .unwrap_or_else(|| get_claude_api_format(&ctx.provider))
        .to_string();
    let response = result.response;

    // Check whether format conversion is needed (OpenRouter and other relays)
    let adapter = get_adapter(&AppType::Claude);
    let needs_transform = adapter.needs_transform(&ctx.provider);

    // Claude only: format conversion
    if needs_transform {
        return handle_claude_transform(response, &ctx, &state, &body, is_stream, &api_format)
            .await;
    }

    // Shared response handling (passthrough)
    process_response(response, &ctx, &state, &CLAUDE_PARSER_CONFIG).await
}

/// Claude format conversion (Claude only)
///
/// Converts from both OpenAI Chat Completions and Responses API formats
async fn handle_claude_transform(
    response: super::hyper_client::ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    _original_body: &Value,
    is_stream: bool,
    api_format: &str,
) -> Result<axum::response::Response, ProxyError> {
    let status = response.status();

    if is_stream {
        // Pick the streaming converter by api_format
        let stream = response.bytes_stream();
        let sse_stream: Box<
            dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin,
        > = if api_format == "openai_responses" {
            Box::new(Box::pin(create_anthropic_sse_stream_from_responses(stream)))
        } else {
            Box::new(Box::pin(create_anthropic_sse_stream(stream)))
        };

        // Create the usage collector
        let usage_collector = {
            let state = state.clone();
            let provider_id = ctx.provider.id.clone();
            let model = ctx.request_model.clone();
            let status_code = status.as_u16();
            let start_time = ctx.start_time;

            SseUsageCollector::new(start_time, move |events, first_token_ms| {
                if let Some(usage) = TokenUsage::from_claude_stream_events(&events) {
                    let latency_ms = start_time.elapsed().as_millis() as u64;
                    let state = state.clone();
                    let provider_id = provider_id.clone();
                    let model = model.clone();

                    tokio::spawn(async move {
                        log_usage(
                            &state,
                            &provider_id,
                            "claude",
                            &model,
                            &model,
                            usage,
                            latency_ms,
                            first_token_ms,
                            true,
                            status_code,
                        )
                        .await;
                    });
                } else {
                    log::debug!("[Claude] OpenRouter streamed response has no usage stats; skipping usage record");
                }
            })
        };

        // Streaming timeout configuration
        let timeout_config = ctx.streaming_timeout_config();

        let logged_stream = create_logged_passthrough_stream(
            sse_stream,
            "Claude/OpenRouter",
            Some(usage_collector),
            timeout_config,
        );

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "Content-Type",
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            "Cache-Control",
            axum::http::HeaderValue::from_static("no-cache"),
        );
        headers.insert(
            "Connection",
            axum::http::HeaderValue::from_static("keep-alive"),
        );

        let body = axum::body::Body::from_stream(logged_stream);
        return Ok((headers, body).into_response());
    }

    // Non-streaming response conversion (OpenAI/Responses -> Anthropic)
    let body_timeout =
        if ctx.app_config.auto_failover_enabled && ctx.app_config.non_streaming_timeout > 0 {
            std::time::Duration::from_secs(ctx.app_config.non_streaming_timeout as u64)
        } else {
            std::time::Duration::ZERO
        };
    let (mut response_headers, _status, body_bytes) =
        read_decoded_body(response, ctx.tag, body_timeout).await?;

    let body_str = String::from_utf8_lossy(&body_bytes);

    let upstream_response: Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        log::error!("[Claude] Failed to parse upstream response: {e}, body: {body_str}");
        ProxyError::TransformError(format!("Failed to parse upstream response: {e}"))
    })?;

    // Pick the non-streaming converter by api_format
    let anthropic_response = if api_format == "openai_responses" {
        transform_responses::responses_to_anthropic(upstream_response)
    } else {
        transform::openai_to_anthropic(upstream_response)
    }
    .map_err(|e| {
        log::error!("[Claude] Failed to convert response: {e}");
        e
    })?;

    // Record usage
    if let Some(usage) = TokenUsage::from_claude_response(&anthropic_response) {
        let model = anthropic_response
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        let latency_ms = ctx.latency_ms();

        let request_model = ctx.request_model.clone();
        tokio::spawn({
            let state = state.clone();
            let provider_id = ctx.provider.id.clone();
            let model = model.to_string();
            async move {
                log_usage(
                    &state,
                    &provider_id,
                    "claude",
                    &model,
                    &request_model,
                    usage,
                    latency_ms,
                    None,
                    false,
                    status.as_u16(),
                )
                .await;
            }
        });
    }

    // Build the response
    let mut builder = axum::response::Response::builder().status(status);
    strip_entity_headers_for_rebuilt_body(&mut response_headers);

    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }

    builder = builder.header("content-type", "application/json");

    let response_body = serde_json::to_vec(&anthropic_response).map_err(|e| {
        log::error!("[Claude] Failed to serialize response: {e}");
        ProxyError::TransformError(format!("Failed to serialize response: {e}"))
    })?;

    let body = axum::body::Body::from(response_body);
    builder.body(body).map_err(|e| {
        log::error!("[Claude] Failed to build response: {e}");
        ProxyError::Internal(format!("Failed to build response: {e}"))
    })
}

fn endpoint_with_query(uri: &axum::http::Uri, endpoint: &str) -> String {
    match uri.query() {
        Some(query) => format!("{endpoint}?{query}"),
        None => endpoint.to_string(),
    }
}

// ============================================================================
// Codex API handlers
// ============================================================================

/// Handles /v1/chat/completions requests (OpenAI Chat Completions API - Codex CLI)
pub async fn handle_chat_completions(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let uri = parts.uri;
    let headers = parts.headers;
    let extensions = parts.extensions;
    let body_bytes = req_body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ProxyError::Internal(format!("Failed to parse request body: {e}")))?;

    let mut ctx =
        RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex").await?;
    let endpoint = endpoint_with_query(&uri, "/chat/completions");

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let forwarder = ctx.create_forwarder(&state);
    let result = match forwarder
        .forward_with_retry(
            &AppType::Codex,
            &endpoint,
            body,
            headers,
            extensions,
            ctx.get_providers(),
        )
        .await
    {
        Ok(result) => result,
        Err(mut err) => {
            if let Some(provider) = err.provider.take() {
                ctx.provider = provider;
            }
            log_forward_error(&state, &ctx, is_stream, &err.error);
            return Err(err.error);
        }
    };

    ctx.provider = result.provider;
    let response = result.response;

    process_response(response, &ctx, &state, &OPENAI_PARSER_CONFIG).await
}

/// Handles /v1/responses requests (OpenAI Responses API - Codex CLI passthrough)
pub async fn handle_responses(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let uri = parts.uri;
    let headers = parts.headers;
    let extensions = parts.extensions;
    let body_bytes = req_body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();
    let body_bytes = decode_request_body(&headers, body_bytes)?;
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ProxyError::Internal(format!("Failed to parse request body: {e}")))?;

    let mut ctx =
        RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex").await?;
    let endpoint = endpoint_with_query(&uri, "/responses");

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let forwarder = ctx.create_forwarder(&state);
    let result = match forwarder
        .forward_with_retry(
            &AppType::Codex,
            &endpoint,
            body,
            headers,
            extensions,
            ctx.get_providers(),
        )
        .await
    {
        Ok(result) => result,
        Err(mut err) => {
            if let Some(provider) = err.provider.take() {
                ctx.provider = provider;
            }
            log_forward_error(&state, &ctx, is_stream, &err.error);
            return Err(err.error);
        }
    };

    ctx.provider = result.provider;
    let response = result.response;

    process_response(response, &ctx, &state, &CODEX_PARSER_CONFIG).await
}

/// Handles /v1/responses/compact requests (OpenAI Responses Compact API - Codex CLI passthrough)
pub async fn handle_responses_compact(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let uri = parts.uri;
    let headers = parts.headers;
    let extensions = parts.extensions;
    let body_bytes = req_body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();
    let body_bytes = decode_request_body(&headers, body_bytes)?;
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ProxyError::Internal(format!("Failed to parse request body: {e}")))?;

    let mut ctx =
        RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex").await?;
    let endpoint = endpoint_with_query(&uri, "/responses/compact");

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let forwarder = ctx.create_forwarder(&state);
    let result = match forwarder
        .forward_with_retry(
            &AppType::Codex,
            &endpoint,
            body,
            headers,
            extensions,
            ctx.get_providers(),
        )
        .await
    {
        Ok(result) => result,
        Err(mut err) => {
            if let Some(provider) = err.provider.take() {
                ctx.provider = provider;
            }
            log_forward_error(&state, &ctx, is_stream, &err.error);
            return Err(err.error);
        }
    };

    ctx.provider = result.provider;
    let response = result.response;

    process_response(response, &ctx, &state, &CODEX_PARSER_CONFIG).await
}

/// Codex compresses request bodies with zstd when it talks to the ChatGPT
/// backend through its built-in provider. The proxy works on the JSON.
fn decode_request_body(headers: &axum::http::HeaderMap, body: Bytes) -> Result<Bytes, ProxyError> {
    let encoding = headers
        .get(axum::http::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match encoding.as_str() {
        "" | "identity" => Ok(body),
        "zstd" => zstd::stream::decode_all(body.as_ref())
            .map(Bytes::from)
            .map_err(|e| ProxyError::Internal(format!("Failed to decompress request body: {e}"))),
        other => Err(ProxyError::Internal(format!(
            "Unsupported request Content-Encoding: {other}"
        ))),
    }
}

/// Codex tries Responses-over-WebSocket first on its built-in provider. The
/// proxy speaks HTTP only; 426 is the answer that makes Codex fall back to it
/// at once instead of spending its retry budget.
pub async fn handle_codex_websocket_refusal() -> impl IntoResponse {
    (
        StatusCode::UPGRADE_REQUIRED,
        [(axum::http::header::CONTENT_LENGTH, "0")],
    )
}

/// GET requests Codex makes against the ChatGPT backend besides the model
/// call itself (the model list). Served with the selected account's login.
pub async fn handle_codex_backend_get(
    State(state): State<ProxyState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let providers = state
        .provider_router
        .select_providers("codex", None)
        .await
        .map_err(|e| ProxyError::Internal(e.to_string()))?;
    let provider = providers
        .into_iter()
        .next()
        .ok_or(ProxyError::NoAvailableProvider)?;

    let adapter = get_adapter(&AppType::Codex);
    let base_url = adapter.extract_base_url(&provider)?;
    let mut url = format!("{}/{}", base_url.trim_end_matches('/'), path);
    if let Some(query) = request.uri().query() {
        url.push('?');
        url.push_str(query);
    }

    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let mut upstream = super::http_client::get_for_provider(proxy_config).get(&url);
    for (name, value) in request.headers() {
        if matches!(
            name.as_str(),
            "host" | "authorization" | "chatgpt-account-id" | "accept-encoding" | "connection"
        ) {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    if super::codex_pool::is_chatgpt_provider(&provider) {
        super::account_pool::ensure_exit_allowed(
            &state.db,
            &provider,
            super::codex_pool::EXIT_TRACE_URL,
        )
        .await?;
        let credentials = super::codex_pool::credentials_for(&state.db, &provider, false).await?;
        upstream = upstream.bearer_auth(credentials.access_token);
        if let Some(account_id) = credentials.account_id {
            upstream = upstream.header("chatgpt-account-id", account_id);
        }
    } else if let Some(auth) = adapter.extract_auth(&provider) {
        upstream = upstream.bearer_auth(auth.api_key);
    }

    let response = upstream
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .cloned();
    let body = response
        .bytes()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;

    let mut builder = axum::response::Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, content_type);
    }
    builder
        .body(axum::body::Body::from(body))
        .map_err(|e| ProxyError::Internal(e.to_string()))
}

/// Everything else Claude Code sends to `ANTHROPIC_BASE_URL` while an Official
/// account is served through the proxy. Token counting is inference and gets
/// the selected account's login; the rest — `/api/oauth/*` (profile, file
/// transfer) and `/v1/code/*` — is the client's own identity plane and keeps
/// the login Claude Code sent, so it never learns another account's identity.
/// Outside that mode this stays the 404 it always was.
pub async fn handle_claude_passthrough(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let path = request.uri().path().to_string();
    if path.starts_with("/backend-api/") {
        return handle_codex_chatgpt_backend(state, request).await;
    }
    let claude_path = path.starts_with("/api/")
        || path.starts_with("/v1/code/")
        || path.starts_with("/v1/messages/");
    if !claude_path {
        return Ok((StatusCode::NOT_FOUND, "Not Found").into_response());
    }

    let providers = state
        .provider_router
        .select_providers("claude", None)
        .await
        .map_err(|e| ProxyError::Internal(e.to_string()))?;
    let Some(provider) = providers.into_iter().next() else {
        return Ok((StatusCode::NOT_FOUND, "Not Found").into_response());
    };
    if !super::providers::ClaudeAdapter::serves_captured_login(&provider) {
        return Ok((StatusCode::NOT_FOUND, "Not Found").into_response());
    }

    super::account_pool::ensure_exit_allowed(
        &state.db,
        &provider,
        super::claude_pool::EXIT_TRACE_URL,
    )
    .await?;

    let presents_pool_login = path.starts_with("/v1/messages/");
    let mut url = format!("{}{path}", super::claude_pool::ANTHROPIC_BASE_URL);
    if let Some(query) = request.uri().query() {
        url.push('?');
        url.push_str(query);
    }

    let (parts, body) = request.into_parts();
    let body_bytes = body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();

    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let mut upstream = super::http_client::get_for_provider(proxy_config)
        .request(parts.method.clone(), &url)
        .body(body_bytes);
    for (name, value) in &parts.headers {
        if matches!(
            name.as_str(),
            "host" | "content-length" | "accept-encoding" | "connection" | "transfer-encoding"
        ) || (presents_pool_login && name.as_str() == "authorization")
        {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    if presents_pool_login {
        let token = super::claude_pool::access_token_for(&provider, false).await?;
        upstream = upstream.bearer_auth(token);
    }

    let response = upstream
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;
    if presents_pool_login {
        super::claude_pool::record_quota(&provider.id, None, response.headers());
    }

    let mut builder = axum::response::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "content-encoding"
        ) {
            continue;
        }
        builder = builder.header(name, value);
    }
    let body = axum::body::Body::from_stream(response.bytes_stream());
    builder
        .body(body)
        .map_err(|e| ProxyError::Internal(e.to_string()))
}

/// Codex's calls to the ChatGPT backend beyond the model, while its
/// `chatgpt_base_url` points here. Its usage (`/wham/usage`, what `/status`
/// shows) is read with the login of the account the proxy serves, so `/status`
/// in an open session shows that account's limits right after a switch. The
/// rest keeps the login Codex sent and reaches ChatGPT as it would have.
async fn handle_codex_chatgpt_backend(
    state: ProxyState,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let path = request.uri().path().to_string();
    let mut url = format!("{}{path}", super::codex_pool::CHATGPT_ORIGIN);
    if let Some(query) = request.uri().query() {
        url.push('?');
        url.push_str(query);
    }

    let served = if path.starts_with("/backend-api/wham/usage") {
        state
            .provider_router
            .select_providers("codex", None)
            .await
            .map_err(|e| ProxyError::Internal(e.to_string()))?
            .into_iter()
            .next()
            .filter(super::codex_pool::is_chatgpt_provider)
    } else {
        None
    };

    let (parts, body) = request.into_parts();
    let body_bytes = body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();

    let proxy_config = served
        .as_ref()
        .and_then(|p| p.meta.as_ref())
        .and_then(|m| m.proxy_config.as_ref());
    let mut upstream = super::http_client::get_for_provider(proxy_config)
        .request(parts.method.clone(), &url)
        .body(body_bytes);
    for (name, value) in &parts.headers {
        if matches!(
            name.as_str(),
            "host" | "content-length" | "accept-encoding" | "connection" | "transfer-encoding"
        ) || (served.is_some()
            && matches!(name.as_str(), "authorization" | "chatgpt-account-id"))
        {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    if let Some(provider) = served.as_ref() {
        super::account_pool::ensure_exit_allowed(
            &state.db,
            provider,
            super::codex_pool::EXIT_TRACE_URL,
        )
        .await?;
        let credentials = super::codex_pool::credentials_for(&state.db, provider, false).await?;
        upstream = upstream.bearer_auth(credentials.access_token);
        if let Some(account_id) = credentials.account_id {
            upstream = upstream.header("chatgpt-account-id", account_id);
        }
    }

    let response = upstream
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;

    let mut builder = axum::response::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "content-encoding"
        ) {
            continue;
        }
        builder = builder.header(name, value);
    }
    let body = axum::body::Body::from_stream(response.bytes_stream());
    builder
        .body(body)
        .map_err(|e| ProxyError::Internal(e.to_string()))
}

// ============================================================================
// Gemini API handler
// ============================================================================

/// Handles Gemini API requests (passthrough, including query parameters)
pub async fn handle_gemini(
    State(state): State<ProxyState>,
    uri: axum::http::Uri,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let headers = parts.headers;
    let extensions = parts.extensions;
    let body_bytes = req_body
        .collect()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ProxyError::Internal(format!("Failed to parse request body: {e}")))?;

    // Gemini carries the model name in the URI
    let mut ctx = RequestContext::new(&state, &body, &headers, AppType::Gemini, "Gemini", "gemini")
        .await?
        .with_model_from_uri(&uri);

    // Extract the full path and query parameters
    let endpoint = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(uri.path());

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let forwarder = ctx.create_forwarder(&state);
    let result = match forwarder
        .forward_with_retry(
            &AppType::Gemini,
            endpoint,
            body,
            headers,
            extensions,
            ctx.get_providers(),
        )
        .await
    {
        Ok(result) => result,
        Err(mut err) => {
            if let Some(provider) = err.provider.take() {
                ctx.provider = provider;
            }
            log_forward_error(&state, &ctx, is_stream, &err.error);
            return Err(err.error);
        }
    };

    ctx.provider = result.provider;
    let response = result.response;

    process_response(response, &ctx, &state, &GEMINI_PARSER_CONFIG).await
}

// ============================================================================
// Usage logging (kept for the Claude conversion logic)
// ============================================================================

fn log_forward_error(
    state: &ProxyState,
    ctx: &RequestContext,
    is_streaming: bool,
    error: &ProxyError,
) {
    use super::usage::logger::UsageLogger;

    let logger = UsageLogger::new(&state.db);
    let status_code = map_proxy_error_to_status(error);
    let error_message = get_error_message(error);
    let request_id = uuid::Uuid::new_v4().to_string();

    if let Err(e) = logger.log_error_with_context(
        request_id,
        ctx.provider.id.clone(),
        ctx.app_type_str.to_string(),
        ctx.request_model.clone(),
        status_code,
        error_message,
        ctx.latency_ms(),
        is_streaming,
        Some(ctx.session_id.clone()),
        None,
    ) {
        log::warn!("Failed to log failed request: {e}");
    }
}

/// Records request usage
#[allow(clippy::too_many_arguments)]
async fn log_usage(
    state: &ProxyState,
    provider_id: &str,
    app_type: &str,
    model: &str,
    request_model: &str,
    usage: TokenUsage,
    latency_ms: u64,
    first_token_ms: Option<u64>,
    is_streaming: bool,
    status_code: u16,
) {
    use super::usage::logger::UsageLogger;

    let logger = UsageLogger::new(&state.db);

    let (multiplier, pricing_model_source) =
        logger.resolve_pricing_config(provider_id, app_type).await;
    let pricing_model = if pricing_model_source == "request" {
        request_model
    } else {
        model
    };

    let request_id = uuid::Uuid::new_v4().to_string();

    if let Err(e) = logger.log_with_calculation(
        request_id,
        provider_id.to_string(),
        app_type.to_string(),
        model.to_string(),
        request_model.to_string(),
        pricing_model.to_string(),
        usage,
        multiplier,
        latency_ms,
        first_token_ms,
        status_code,
        None,
        None, // provider_type
        is_streaming,
    ) {
        log::warn!("[USG-001] Failed to record usage: {e}");
    }
}
