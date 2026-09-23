//! Response processor
//!
//! Unified handling of streaming and non-streaming API responses

use super::{
    handler_config::UsageParserConfig,
    handler_context::{RequestContext, StreamingTimeoutConfig},
    hyper_client::ProxyResponse,
    server::ProxyState,
    sse::strip_sse_field,
    usage::parser::TokenUsage,
    ProxyError,
};
use axum::http::header::HeaderMap;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::Value;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

// ============================================================================
// Response decompression
// ============================================================================

/// Decompress response body bytes according to content-encoding
///
/// reqwest's automatic decompression is disabled (to pass accept-encoding through), so decompress manually.
fn decompress_body(content_encoding: &str, body: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    match content_encoding {
        "gzip" | "x-gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(body);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            Ok(decompressed)
        }
        "deflate" => {
            let mut decoder = flate2::read::DeflateDecoder::new(body);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            Ok(decompressed)
        }
        "br" => {
            let mut decompressed = Vec::new();
            brotli::BrotliDecompress(&mut std::io::Cursor::new(body), &mut decompressed)?;
            Ok(decompressed)
        }
        _ => {
            log::warn!("Unknown content-encoding: {content_encoding}, skipping decompression");
            Ok(body.to_vec())
        }
    }
}

/// Extract content-encoding from the response headers (ignoring identity and chunked)
fn get_content_encoding(headers: &HeaderMap) -> Option<String> {
    headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "identity")
}

/// Remove entity headers that become inaccurate once the body is rebuilt.
///
/// A buffered body is sent with a content-length. Leaving the upstream
/// `transfer-encoding: chunked` on that response makes hyper abort the
/// client connection (`user sent unexpected header`) after a successful
/// upstream reply.
pub(crate) fn strip_entity_headers_for_rebuilt_body(headers: &mut HeaderMap) {
    headers.remove(axum::http::header::CONTENT_ENCODING);
    strip_framing_headers(headers);
}

/// Drop hop-by-hop framing. The server sets content-length or chunked
/// encoding for the body it actually sends.
fn strip_framing_headers(headers: &mut HeaderMap) {
    headers.remove(axum::http::header::CONTENT_LENGTH);
    headers.remove(axum::http::header::TRANSFER_ENCODING);
    headers.remove(axum::http::header::CONNECTION);
}

/// Read the response body and decompress it when needed, so headers match the returned body.
///
/// `body_timeout`: whole-body timeout. When non-zero, `.bytes()` is wrapped in `tokio::time::timeout`
/// so an upstream that stalls the body after sending headers cannot hang the request forever.
/// Pass `Duration::ZERO` to disable the timeout (when failover is off).
pub(crate) async fn read_decoded_body(
    response: ProxyResponse,
    tag: &str,
    body_timeout: Duration,
) -> Result<(HeaderMap, http::StatusCode, Bytes), ProxyError> {
    let mut headers = response.headers().clone();
    let status = response.status();
    let raw_bytes = if body_timeout.is_zero() {
        response.bytes().await?
    } else {
        tokio::time::timeout(body_timeout, response.bytes())
            .await
            .map_err(|_| {
                ProxyError::Timeout(format!(
                    "Timed out reading the response body after {}s (the upstream sent its headers but no body)",
                    body_timeout.as_secs()
                ))
            })??
    };

    log::debug!(
        "[{tag}] Received upstream response body: status={}, bytes={}, headers={}",
        status.as_u16(),
        raw_bytes.len(),
        format_headers(&headers)
    );

    let mut body_bytes = raw_bytes.clone();
    let mut decoded = false;

    if let Some(encoding) = get_content_encoding(&headers) {
        log::debug!("[{tag}] Decompressing non-streaming response: content-encoding={encoding}");
        match decompress_body(&encoding, &raw_bytes) {
            Ok(decompressed) => {
                body_bytes = Bytes::from(decompressed);
                decoded = true;
            }
            Err(e) => {
                log::warn!("[{tag}] Decompression failed ({encoding}): {e}, using raw data");
            }
        }
    }

    // The bytes above are the full payload. Upstream `transfer-encoding:
    // chunked` must not be copied onto that response.
    if decoded {
        headers.remove(axum::http::header::CONTENT_ENCODING);
    }
    strip_framing_headers(&mut headers);

    Ok((headers, status, body_bytes))
}

// ============================================================================
// Public interface
// ============================================================================

/// Whether the response is an SSE stream
#[inline]
pub fn is_sse_response(response: &ProxyResponse) -> bool {
    response.is_sse()
}

/// Handle a streaming response
pub async fn handle_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
) -> Response {
    let status = response.status();
    log::debug!(
        "[{}] Received upstream streaming response: status={}, headers={}",
        ctx.tag,
        status.as_u16(),
        format_headers(response.headers())
    );
    // Check whether the stream is compressed (SSE is normally uncompressed; if compressed, SSE parsing fails)
    if let Some(encoding) = get_content_encoding(response.headers()) {
        log::warn!(
            "[{}] Streaming response has content-encoding={encoding}; SSE parsing may fail. \
             The upstream compressed the SSE stream after accept-encoding passthrough.",
            ctx.tag
        );
    }

    let mut builder = axum::response::Response::builder().status(status);

    // Hop-by-hop headers describe the upstream connection. Hyper frames
    // this stream itself; copying `transfer-encoding` aborts the client.
    for (key, value) in response.headers() {
        if matches!(
            key,
            &axum::http::header::TRANSFER_ENCODING
                | &axum::http::header::CONNECTION
                | &axum::http::header::CONTENT_LENGTH
        ) {
            continue;
        }
        builder = builder.header(key, value);
    }

    // Create the byte stream
    let stream = response.bytes_stream();

    // Create the usage collector
    let usage_collector = create_usage_collector(ctx, state, status.as_u16(), parser_config);

    // Get the streaming timeout config
    let timeout_config = ctx.streaming_timeout_config();

    // Create the passthrough stream with logging and timeouts
    let logged_stream =
        create_logged_passthrough_stream(stream, ctx.tag, Some(usage_collector), timeout_config);

    let body = axum::body::Body::from_stream(logged_stream);
    match builder.body(body) {
        Ok(resp) => resp,
        Err(e) => {
            log::error!("[{}] Failed to build streaming response: {e}", ctx.tag);
            ProxyError::Internal(format!("Failed to build streaming response: {e}")).into_response()
        }
    }
}

/// Handle a non-streaming response
pub async fn handle_non_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
) -> Result<Response, ProxyError> {
    // Whole-body timeout: only when failover is on and the configured value is non-zero
    let body_timeout =
        if ctx.app_config.auto_failover_enabled && ctx.app_config.non_streaming_timeout > 0 {
            Duration::from_secs(ctx.app_config.non_streaming_timeout as u64)
        } else {
            Duration::ZERO
        };
    let (response_headers, status, body_bytes) =
        read_decoded_body(response, ctx.tag, body_timeout).await?;

    log::debug!(
        "[{}] Upstream response body: {}",
        ctx.tag,
        String::from_utf8_lossy(&body_bytes)
    );

    // Parse and record usage
    if let Ok(json_value) = serde_json::from_slice::<Value>(&body_bytes) {
        // Parse usage
        if let Some(usage) = (parser_config.response_parser)(&json_value) {
            // Prefer the model name parsed from usage, then the response's model field, then the request model
            let model = if let Some(ref m) = usage.model {
                m.clone()
            } else if let Some(m) = json_value.get("model").and_then(|m| m.as_str()) {
                m.to_string()
            } else {
                ctx.request_model.clone()
            };

            spawn_log_usage(
                state,
                ctx,
                usage,
                &model,
                &ctx.request_model,
                status.as_u16(),
                false,
            );
        } else {
            let model = json_value
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or(&ctx.request_model)
                .to_string();
            spawn_log_usage(
                state,
                ctx,
                TokenUsage::default(),
                &model,
                &ctx.request_model,
                status.as_u16(),
                false,
            );
            log::debug!(
                "[{}] Could not parse usage info, skipping record",
                parser_config.app_type_str
            );
        }
    } else {
        log::debug!(
            "[{}] <<< Response (non-JSON): {} bytes",
            ctx.tag,
            body_bytes.len()
        );
        spawn_log_usage(
            state,
            ctx,
            TokenUsage::default(),
            &ctx.request_model,
            &ctx.request_model,
            status.as_u16(),
            false,
        );
    }

    // Build the response
    let mut builder = axum::response::Response::builder().status(status);
    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }

    let body = axum::body::Body::from(body_bytes);
    builder.body(body).map_err(|e| {
        log::error!("[{}] Failed to build response: {e}", ctx.tag);
        ProxyError::Internal(format!("Failed to build response: {e}"))
    })
}

/// Common response handling entry point
///
/// Picks streaming or non-streaming handling by response type
pub async fn process_response(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
) -> Result<Response, ProxyError> {
    if is_sse_response(&response) {
        Ok(handle_streaming(response, ctx, state, parser_config).await)
    } else {
        handle_non_streaming(response, ctx, state, parser_config).await
    }
}

// ============================================================================
// SSE usage collector
// ============================================================================

type UsageCallbackWithTiming = Arc<dyn Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static>;

/// SSE usage collector
#[derive(Clone)]
pub struct SseUsageCollector {
    inner: Arc<SseUsageCollectorInner>,
}

struct SseUsageCollectorInner {
    events: Mutex<Vec<Value>>,
    first_event_time: Mutex<Option<std::time::Instant>>,
    start_time: std::time::Instant,
    on_complete: UsageCallbackWithTiming,
    finished: AtomicBool,
}

impl SseUsageCollector {
    /// Create a usage collector
    pub fn new(
        start_time: std::time::Instant,
        callback: impl Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static,
    ) -> Self {
        let on_complete: UsageCallbackWithTiming = Arc::new(callback);
        Self {
            inner: Arc::new(SseUsageCollectorInner {
                events: Mutex::new(Vec::new()),
                first_event_time: Mutex::new(None),
                start_time,
                on_complete,
                finished: AtomicBool::new(false),
            }),
        }
    }

    /// Push an SSE event
    pub async fn push(&self, event: Value) {
        // Record the time of the first event
        {
            let mut first_time = self.inner.first_event_time.lock().await;
            if first_time.is_none() {
                *first_time = Some(std::time::Instant::now());
            }
        }
        let mut events = self.inner.events.lock().await;
        events.push(event);
    }

    /// Finish collecting and fire the callback
    pub async fn finish(&self) {
        if self.inner.finished.swap(true, Ordering::SeqCst) {
            return;
        }

        let events = {
            let mut guard = self.inner.events.lock().await;
            std::mem::take(&mut *guard)
        };

        let first_token_ms = {
            let first_time = self.inner.first_event_time.lock().await;
            first_time.map(|t| (t - self.inner.start_time).as_millis() as u64)
        };

        (self.inner.on_complete)(events, first_token_ms);
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Create a usage collector
fn create_usage_collector(
    ctx: &RequestContext,
    state: &ProxyState,
    status_code: u16,
    parser_config: &UsageParserConfig,
) -> SseUsageCollector {
    let logging_enabled = state
        .config
        .try_read()
        .map(|c| c.enable_logging)
        .unwrap_or(true);
    let state = state.clone();
    let provider_id = ctx.provider.id.clone();
    let request_model = ctx.request_model.clone();
    let app_type_str = parser_config.app_type_str;
    let tag = ctx.tag;
    let start_time = ctx.start_time;
    let stream_parser = parser_config.stream_parser;
    let model_extractor = parser_config.model_extractor;
    let session_id = ctx.session_id.clone();

    SseUsageCollector::new(start_time, move |events, first_token_ms| {
        if !logging_enabled {
            return;
        }
        if let Some(usage) = stream_parser(&events) {
            let model = model_extractor(&events, &request_model);
            let latency_ms = start_time.elapsed().as_millis() as u64;

            let state = state.clone();
            let provider_id = provider_id.clone();
            let session_id = session_id.clone();
            let request_model = request_model.clone();

            tokio::spawn(async move {
                log_usage_internal(
                    &state,
                    &provider_id,
                    app_type_str,
                    &model,
                    &request_model,
                    usage,
                    latency_ms,
                    first_token_ms,
                    true, // is_streaming
                    status_code,
                    Some(session_id),
                )
                .await;
            });
        } else {
            let model = model_extractor(&events, &request_model);
            let latency_ms = start_time.elapsed().as_millis() as u64;
            let state = state.clone();
            let provider_id = provider_id.clone();
            let session_id = session_id.clone();
            let request_model = request_model.clone();

            tokio::spawn(async move {
                log_usage_internal(
                    &state,
                    &provider_id,
                    app_type_str,
                    &model,
                    &request_model,
                    TokenUsage::default(),
                    latency_ms,
                    first_token_ms,
                    true, // is_streaming
                    status_code,
                    Some(session_id),
                )
                .await;
            });
            log::debug!("[{tag}] Streaming response has no usage stats, skipping usage record");
        }
    })
}

/// Record usage asynchronously
fn spawn_log_usage(
    state: &ProxyState,
    ctx: &RequestContext,
    usage: TokenUsage,
    model: &str,
    request_model: &str,
    status_code: u16,
    is_streaming: bool,
) {
    // Check enable_logging before spawning the log task
    if let Ok(config) = state.config.try_read() {
        if !config.enable_logging {
            return;
        }
    }

    let state = state.clone();
    let provider_id = ctx.provider.id.clone();
    let app_type_str = ctx.app_type_str.to_string();
    let model = model.to_string();
    let request_model = request_model.to_string();
    let latency_ms = ctx.latency_ms();
    let session_id = ctx.session_id.clone();

    tokio::spawn(async move {
        log_usage_internal(
            &state,
            &provider_id,
            &app_type_str,
            &model,
            &request_model,
            usage,
            latency_ms,
            None,
            is_streaming,
            status_code,
            Some(session_id),
        )
        .await;
    });
}

/// Internal usage recording
#[allow(clippy::too_many_arguments)]
async fn log_usage_internal(
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
    session_id: Option<String>,
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

    log::debug!(
        "[{app_type}] Recording request log: id={request_id}, provider={provider_id}, model={model}, streaming={is_streaming}, status={status_code}, latency_ms={latency_ms}, first_token_ms={first_token_ms:?}, session={}, input={}, output={}, cache_read={}, cache_creation={}",
        session_id.as_deref().unwrap_or("none"),
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_creation_tokens
    );

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
        session_id,
        None, // provider_type
        is_streaming,
    ) {
        log::warn!("[USG-001] Failed to record usage: {e}");
    }
}

/// Create a passthrough stream with logging and timeout control
pub fn create_logged_passthrough_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    tag: &'static str,
    usage_collector: Option<SseUsageCollector>,
    timeout_config: StreamingTimeoutConfig,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut collector = usage_collector;
        let mut is_first_chunk = true;

        // Timeout config
        let first_byte_timeout = if timeout_config.first_byte_timeout > 0 {
            Some(Duration::from_secs(timeout_config.first_byte_timeout))
        } else {
            None
        };
        let idle_timeout = if timeout_config.idle_timeout > 0 {
            Some(Duration::from_secs(timeout_config.idle_timeout))
        } else {
            None
        };

        tokio::pin!(stream);

        loop {
            // Pick the timeout: first-byte timeout or idle timeout
            let timeout_duration = if is_first_chunk {
                first_byte_timeout
            } else {
                idle_timeout
            };

            let chunk_result = match timeout_duration {
                Some(duration) => {
                    match tokio::time::timeout(duration, stream.next()).await {
                        Ok(Some(chunk)) => Some(chunk),
                        Ok(None) => None, // stream ended
                        Err(_) => {
                            // Timed out
                            let timeout_type = if is_first_chunk { "first byte" } else { "idle" };
                            log::error!("[{tag}] the streamed response timed out ({timeout_type}, {}s)", duration.as_secs());
                            yield Err(std::io::Error::other(format!("the streamed response timed out ({timeout_type})")));
                            break;
                        }
                    }
                }
                None => stream.next().await, // no timeout
            };

            match chunk_result {
                Some(Ok(bytes)) => {
                    if is_first_chunk {
                        log::debug!(
                            "[{tag}] Received first upstream stream chunk: bytes={}",
                            bytes.len()
                        );
                    }
                    is_first_chunk = false;
                    let text = String::from_utf8_lossy(&bytes);
                    buffer.push_str(&text);

                    // Try to parse and log complete SSE events
                    while let Some(pos) = buffer.find("\n\n") {
                        let event_text = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if !event_text.trim().is_empty() {
                            // Extract the data part and try to parse it as JSON
                            for line in event_text.lines() {
                                if let Some(data) = strip_sse_field(line, "data") {
                                    if data.trim() != "[DONE]" {
                                        if let Ok(json_value) = serde_json::from_str::<Value>(data) {
                                            if let Some(c) = &collector {
                                                c.push(json_value.clone()).await;
                                            }
                                            log::debug!("[{tag}] <<< SSE event: {data}");
                                        } else {
                                            log::debug!("[{tag}] <<< SSE data: {data}");
                                        }
                                    } else {
                                        log::debug!("[{tag}] <<< SSE: [DONE]");
                                    }
                                }
                            }
                        }
                    }

                    yield Ok(bytes);
                }
                Some(Err(e)) => {
                    log::error!("[{tag}] Stream error: {e}");
                    yield Err(std::io::Error::other(e.to_string()));
                    break;
                }
                None => {
                    // Stream ended normally
                    break;
                }
            }
        }

        if let Some(c) = collector.take() {
            c.finish().await;
        }
    }
}

fn format_headers(headers: &HeaderMap) -> String {
    headers
        .iter()
        .map(|(key, value)| {
            let value_str = value.to_str().unwrap_or("<non-utf8>");
            format!("{key}={value_str}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::error::AppError;
    use crate::provider::ProviderMeta;
    use crate::proxy::failover_switch::FailoverSwitchManager;
    use crate::proxy::provider_router::ProviderRouter;
    use crate::proxy::types::{ProxyConfig, ProxyStatus};
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[test]
    fn test_strip_sse_field_accepts_optional_space() {
        assert_eq!(
            super::strip_sse_field("data: {\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("data:{\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("event: message_start", "event"),
            Some("message_start")
        );
        assert_eq!(
            super::strip_sse_field("event:message_start", "event"),
            Some("message_start")
        );
        assert_eq!(super::strip_sse_field("id:1", "data"), None);
    }

    fn build_state(db: Arc<Database>) -> ProxyState {
        ProxyState {
            db: db.clone(),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
            provider_router: Arc::new(ProviderRouter::new(db.clone())),
            app_handle: None,
            failover_manager: Arc::new(FailoverSwitchManager::new(db)),
        }
    }

    fn seed_pricing(db: &Database) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["resp-model", "Resp Model", "1.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["req-model", "Req Model", "2.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn insert_provider(
        db: &Database,
        id: &str,
        app_type: &str,
        meta: ProviderMeta,
    ) -> Result<(), AppError> {
        let meta_json =
            serde_json::to_string(&meta).map_err(|e| AppError::Database(e.to_string()))?;
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, app_type, "Test Provider", "{}", meta_json],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    #[tokio::test]
    async fn test_log_usage_uses_provider_override_config() -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "claude";

        db.set_default_cost_multiplier(app_type, "1.5").await?;
        db.set_pricing_model_source(app_type, "response").await?;
        seed_pricing(&db)?;

        let mut meta = ProviderMeta::default();
        meta.cost_multiplier = Some("2".to_string());
        meta.pricing_model_source = Some("request".to_string());
        insert_provider(&db, "provider-1", app_type, meta)?;

        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
        };

        log_usage_internal(
            &state,
            "provider-1",
            app_type,
            "resp-model",
            "req-model",
            usage,
            10,
            None,
            false,
            200,
            None,
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (model, request_model, total_cost, cost_multiplier): (String, String, String, String) =
            conn.query_row(
                "SELECT model, request_model, total_cost_usd, cost_multiplier
                 FROM proxy_request_logs WHERE provider_id = ?1",
                ["provider-1"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        assert_eq!(model, "resp-model");
        assert_eq!(request_model, "req-model");
        assert_eq!(
            Decimal::from_str(&cost_multiplier).unwrap(),
            Decimal::from_str("2").unwrap()
        );
        assert_eq!(
            Decimal::from_str(&total_cost).unwrap(),
            Decimal::from_str("4").unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_log_usage_falls_back_to_global_defaults() -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "claude";

        db.set_default_cost_multiplier(app_type, "1.5").await?;
        db.set_pricing_model_source(app_type, "response").await?;
        seed_pricing(&db)?;

        let meta = ProviderMeta::default();
        insert_provider(&db, "provider-2", app_type, meta)?;

        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
        };

        log_usage_internal(
            &state,
            "provider-2",
            app_type,
            "resp-model",
            "req-model",
            usage,
            10,
            None,
            false,
            200,
            None,
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (total_cost, cost_multiplier): (String, String) = conn
            .query_row(
                "SELECT total_cost_usd, cost_multiplier
                 FROM proxy_request_logs WHERE provider_id = ?1",
                ["provider-2"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        assert_eq!(
            Decimal::from_str(&cost_multiplier).unwrap(),
            Decimal::from_str("1.5").unwrap()
        );
        assert_eq!(
            Decimal::from_str(&total_cost).unwrap(),
            Decimal::from_str("1.5").unwrap()
        );
        Ok(())
    }
}
