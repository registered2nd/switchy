//! Codex's Responses-over-WebSocket transport, relayed to the ChatGPT backend.
//!
//! Codex (0.160 and later) opens one WebSocket to `/backend-api/codex/responses`
//! and keeps it across turns, sending later turns as increments of earlier
//! ones (`previous_response_id`), which only that connection can resolve. The
//! account is therefore chosen when the connection opens. When the account the
//! pool would choose changes, or the serving account is refused for usage, the
//! relay closes the connection once no turn is in flight; Codex reconnects on
//! its next turn with a full request, and the router picks the account again.
//!
//! As on the HTTP path, an account that refuses the connection, or refuses a
//! turn for usage before any of the turn reached Codex, is passed over for the
//! next one; the turn is sent again there and Codex sees no error.
//!
//! Anything the relay cannot serve (a provider that is not a ChatGPT login, or
//! no account accepting the connection) is answered 426, which makes Codex use
//! the HTTP transport for the rest of its session.

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{
    frame::coding::CloseCode, CloseFrame, Role, WebSocketConfig,
};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::server::ProxyState;
use crate::app_config::AppType;
use crate::database::SwitchReason;
use crate::provider::Provider;

/// How often an idle connection checks whether the pool would still choose
/// its account.
const ACCOUNT_CHECK: Duration = Duration::from_secs(3);

/// Handshake headers the relay sets itself on each side, and headers that
/// name the client's own login rather than the account presented.
const NOT_FORWARDED: &[&str] = &[
    "host",
    "connection",
    "upgrade",
    "content-length",
    "authorization",
    "chatgpt-account-id",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-accept",
    // Neither side of the relay compresses frames.
    "sec-websocket-extensions",
];

type Upstream = WebSocketStream<reqwest::Upgraded>;

/// No size limits of the relay's own: a long conversation's full request runs
/// to tens of megabytes in one frame, and the backend decides what it accepts.
fn unlimited() -> Option<WebSocketConfig> {
    Some(
        WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None),
    )
}

fn fall_back_to_http() -> Response {
    (
        StatusCode::UPGRADE_REQUIRED,
        [(header::CONTENT_LENGTH, "0")],
    )
        .into_response()
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// The ChatGPT accounts the pool would serve Codex from now, in order. Empty
/// when its first choice is not a ChatGPT login: that is served over HTTP.
async fn chosen_accounts(state: &ProxyState) -> Vec<Provider> {
    let Ok(providers) = state.provider_router.select_providers("codex", None).await else {
        return Vec::new();
    };
    if !providers
        .first()
        .is_some_and(super::codex_pool::is_chatgpt_provider)
    {
        return Vec::new();
    }
    providers
        .into_iter()
        .filter(super::codex_pool::is_chatgpt_provider)
        .collect()
}

/// Why the pool chose an account other than Codex's current provider.
fn rotation_reason(state: &ProxyState) -> (SwitchReason, String) {
    let current = crate::settings::get_effective_current_provider(&state.db, &AppType::Codex)
        .ok()
        .flatten();
    let threshold = state
        .db
        .get_account_pool_config()
        .unwrap_or_default()
        .threshold_percent;
    match current {
        Some(current) => super::account_pool::rotation_reason(&current, None, threshold),
        None => (SwitchReason::Rotation, "Near its usage limit".to_string()),
    }
}

/// Records in the switch history, the window and the tray that `provider` now
/// serves Codex, when it is not the current provider.
fn note_switch(state: &ProxyState, provider: &Provider, reason: (SwitchReason, String)) {
    let current = crate::settings::get_effective_current_provider(&state.db, &AppType::Codex)
        .ok()
        .flatten();
    if current.as_deref() == Some(provider.id.as_str()) {
        return;
    }
    let (fm, app, id, name) = (
        state.failover_manager.clone(),
        state.app_handle.clone(),
        provider.id.clone(),
        provider.name.clone(),
    );
    tokio::spawn(async move {
        let _ = fm
            .try_switch(app.as_ref(), "codex", &id, &name, Some(reason))
            .await;
    });
}

/// The client's handshake, kept to open the upstream again on another account.
/// Held as owned parts: the request body is not Sync, so the request itself
/// cannot be borrowed across an await.
#[derive(Clone)]
struct ClientHandshake {
    headers: HeaderMap,
    query: Option<String>,
}

/// Opens the upstream WebSocket on the first of `accounts` that accepts it,
/// as the HTTP path tries the next account when one fails. `passed_over` is
/// why the pool moved past the current provider, when it did.
async fn connect_first(
    state: &ProxyState,
    accounts: Vec<Provider>,
    handshake: &ClientHandshake,
    mut passed_over: Option<(SwitchReason, String)>,
) -> Option<(Provider, Upstream, HeaderMap)> {
    for provider in accounts {
        match connect_upstream(state, &provider, handshake).await {
            Ok((upstream, headers)) => {
                super::codex_pool::record_quota(&provider.id, &headers);
                let reason = passed_over.unwrap_or_else(|| rotation_reason(state));
                note_switch(state, &provider, reason);
                return Some((provider, upstream, headers));
            }
            Err((status, e)) => {
                log::warn!("[CodexWS] provider={}: websocket refused ({e})", provider.id);
                if passed_over.is_none() {
                    let reason = if super::codex_pool::needs_sign_in(&provider) {
                        SwitchReason::SignedOut
                    } else if status == Some(429) {
                        SwitchReason::Limit
                    } else {
                        SwitchReason::Failover
                    };
                    passed_over = Some((reason, e));
                }
            }
        }
    }
    None
}

pub async fn handle(
    axum::extract::State(state): axum::extract::State<ProxyState>,
    mut request: axum::extract::Request,
) -> Response {
    if !is_websocket_upgrade(request.headers()) {
        return fall_back_to_http();
    }
    let Some(client_key) = request
        .headers()
        .get(header::SEC_WEBSOCKET_KEY)
        .map(|v| v.as_bytes().to_vec())
    else {
        return fall_back_to_http();
    };
    let accounts = chosen_accounts(&state).await;
    if accounts.is_empty() {
        return fall_back_to_http();
    }

    let handshake = ClientHandshake {
        headers: request.headers().clone(),
        query: request.uri().query().map(str::to_string),
    };
    let Some((provider, upstream, upstream_headers)) =
        connect_first(&state, accounts, &handshake, None).await
    else {
        log::warn!("[CodexWS] no account accepted the websocket; Codex falls back to HTTP");
        return fall_back_to_http();
    };
    log::info!(
        "[CodexWS] relaying a Codex websocket through provider={}",
        provider.id
    );

    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let client =
                    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, unlimited())
                        .await;
                relay(state, provider, client, upstream, handshake).await;
            }
            Err(e) => log::warn!("[CodexWS] client upgrade failed: {e}"),
        }
    });

    let mut response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_ACCEPT, derive_accept_key(&client_key));
    // What Codex reads from the handshake: turn state, model, quota.
    for (name, value) in upstream_headers.iter() {
        let name_str = name.as_str();
        if NOT_FORWARDED.contains(&name_str)
            || matches!(
                name_str,
                "transfer-encoding" | "content-type" | "date" | "server" | "set-cookie"
            )
            || name_str.starts_with("cf-")
        {
            continue;
        }
        response = response.header(name, value);
    }
    response
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| fall_back_to_http())
}

/// Opens the WebSocket to the ChatGPT backend with `provider`'s login, after
/// the exit check. Returns the upstream handshake's response headers, or the
/// status the upstream refused it with.
async fn connect_upstream(
    state: &ProxyState,
    provider: &Provider,
    handshake: &ClientHandshake,
) -> Result<(Upstream, HeaderMap), (Option<u16>, String)> {
    super::account_pool::ensure_exit_allowed(
        &state.db,
        provider,
        super::codex_pool::EXIT_TRACE_URL,
    )
    .await
    .map_err(|e| (None, e.to_string()))?;
    let credentials = super::codex_pool::credentials_for(&state.db, provider, false)
        .await
        .map_err(|e| (None, e.to_string()))?;

    let mut url = format!("{}/responses", super::codex_pool::CHATGPT_CODEX_BASE_URL);
    if let Some(query) = handshake.query.as_deref() {
        url.push('?');
        url.push_str(query);
    }
    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let client = super::http_client::get_http1_for_provider(proxy_config).map_err(|e| (None, e))?;

    let mut upstream = client
        .get(&url)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, generate_key())
        .bearer_auth(&credentials.access_token);
    if let Some(account_id) = credentials.account_id.as_deref() {
        upstream = upstream.header("chatgpt-account-id", account_id);
    }
    for (name, value) in &handshake.headers {
        if !NOT_FORWARDED.contains(&name.as_str()) {
            upstream = upstream.header(name, value);
        }
    }

    let response = upstream
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| (None, e.to_string()))?;
    let status = response.status();
    if status != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        let body = response.text().await.unwrap_or_default();
        super::codex_pool::record_limit_refusal(&provider.id, status.as_u16(), Some(&body));
        return Err((
            Some(status.as_u16()),
            format!(
                "upstream answered {status}: {}",
                body.chars().take(300).collect::<String>()
            ),
        ));
    }
    let headers = response.headers().clone();
    let upgraded = response
        .upgrade()
        .await
        .map_err(|e| (None, e.to_string()))?;
    let socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, unlimited()).await;
    Ok((socket, headers))
}

/// The event type of a JSON text frame.
fn event_type(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value.get("type")?.as_str().map(str::to_string)
}

/// Whether an upstream event ends the turn in flight.
fn ends_turn(kind: &str) -> bool {
    matches!(
        kind,
        "response.completed" | "response.failed" | "response.incomplete" | "error"
    )
}

/// Events that open a turn before any of the answer. They are held back until
/// the turn is known not to be refused, so a refusal can move to another
/// account without Codex having seen the turn start.
fn opens_turn(kind: &str) -> bool {
    matches!(
        kind,
        "response.created"
            | "response.in_progress"
            | "codex.rate_limits"
            | "codex.response.metadata"
            | "responsesapi.websocket_timing"
    )
}

/// The status of a wrapped error event (`{"type":"error","status":429,...}`)
/// and the `x-codex-*` quota it carries, as headers.
fn error_event(text: &str) -> Option<(u16, HeaderMap)> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "error" {
        return None;
    }
    let status = value
        .get("status")
        .or_else(|| value.get("status_code"))?
        .as_u64()? as u16;
    let mut headers = HeaderMap::new();
    for (name, value) in value
        .get("headers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let value = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            headers.insert(name, value);
        }
    }
    Some((status, headers))
}

/// What the usage log needs of the turn in flight.
struct TurnLog {
    started: std::time::Instant,
    first_event_ms: Option<u64>,
    request_model: String,
    session_id: String,
    /// Codex's connection warm-up (`generate: false`): no answer, not counted.
    warmup: bool,
}

impl TurnLog {
    fn new(create: &str, handshake: &ClientHandshake) -> Self {
        let body: Value = serde_json::from_str(create).unwrap_or(Value::Null);
        Self {
            started: std::time::Instant::now(),
            first_event_ms: None,
            request_model: body
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            session_id: super::session::extract_session_id(&handshake.headers, &body, "codex")
                .session_id,
            warmup: body.get("generate").and_then(Value::as_bool) == Some(false),
        }
    }
}

fn logging_enabled(state: &ProxyState) -> bool {
    state
        .config
        .try_read()
        .map(|c| c.enable_logging)
        .unwrap_or(true)
}

/// Records a finished turn in the request log, as the HTTP path records a
/// streamed response, so Usage Statistics count it for `provider`.
fn log_completed(state: &ProxyState, provider: &Provider, turn: &TurnLog, event: &str) {
    if turn.warmup || !logging_enabled(state) {
        return;
    }
    let Ok(event) = serde_json::from_str::<Value>(event) else {
        return;
    };
    let Some(usage) =
        super::usage::parser::TokenUsage::from_codex_stream_events_auto(std::slice::from_ref(&event))
    else {
        return;
    };
    let model = usage
        .model
        .clone()
        .or_else(|| {
            event
                .pointer("/response/model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| turn.request_model.clone());
    let (state, provider_id, request_model, session_id) = (
        state.clone(),
        provider.id.clone(),
        turn.request_model.clone(),
        turn.session_id.clone(),
    );
    let latency_ms = turn.started.elapsed().as_millis() as u64;
    let first_event_ms = turn.first_event_ms;
    tokio::spawn(async move {
        super::response_processor::log_usage_internal(
            &state,
            &provider_id,
            "codex",
            &model,
            &request_model,
            usage,
            latency_ms,
            first_event_ms,
            true,
            200,
            Some(session_id),
        )
        .await;
    });
}

/// Records a turn the backend refused, against the account that refused it.
fn log_refused(state: &ProxyState, provider: &Provider, turn: &TurnLog, status: u16, event: &str) {
    if !logging_enabled(state) {
        return;
    }
    let logger = super::usage::logger::UsageLogger::new(&state.db);
    if let Err(e) = logger.log_error_with_context(
        uuid::Uuid::new_v4().to_string(),
        provider.id.clone(),
        "codex".to_string(),
        turn.request_model.clone(),
        status,
        event.chars().take(500).collect(),
        turn.started.elapsed().as_millis() as u64,
        true,
        Some(turn.session_id.clone()),
        None,
    ) {
        log::warn!("[CodexWS] could not record a refused turn: {e}");
    }
}

/// Moves frames both ways until either side closes, or until the pool would
/// serve another account and no turn is in flight. A turn refused for usage
/// before any of it reached Codex is sent again on the next account.
async fn relay(
    state: ProxyState,
    mut provider: Provider,
    client: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    upstream: Upstream,
    handshake: ClientHandshake,
) {
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    // The turn in flight, and whether any of its events have reached Codex;
    // those that have not are in `held`.
    let mut turn: Option<Message> = None;
    let mut turn_log: Option<TurnLog> = None;
    let mut streaming = false;
    let mut held: Vec<Message> = Vec::new();
    let mut answered = false;
    let mut refused = false;
    let mut check = tokio::time::interval(ACCOUNT_CHECK);
    check.tick().await;

    let reason = 'relay: loop {
        tokio::select! {
            message = client_rx.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => break format!("client error: {e}"),
                    None => break "client closed".to_string(),
                };
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
                if let Message::Text(text) = &message {
                    if event_type(text.as_str()).as_deref() == Some("response.create") {
                        turn = Some(message.clone());
                        turn_log = Some(TurnLog::new(text.as_str(), &handshake));
                        streaming = false;
                        held.clear();
                    }
                }
                let closing = matches!(message, Message::Close(_));
                if upstream_tx.send(message).await.is_err() || closing {
                    break "client closed".to_string();
                }
            }
            message = upstream_rx.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => break format!("upstream error: {e}"),
                    None => break "upstream closed".to_string(),
                };
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
                if let Message::Text(text) = &message {
                    let text = text.as_str().to_string();
                    let kind = event_type(&text).unwrap_or_default();
                    let error = error_event(&text);
                    if let Some((status, quota)) = &error {
                        super::codex_pool::record_quota(&provider.id, quota);
                        super::codex_pool::record_limit_refusal(&provider.id, *status, Some(&text));
                        if let Some(log) = &turn_log {
                            log_refused(&state, &provider, log, *status, &text);
                        }
                    }
                    let limit = matches!(error, Some((429, _)));
                    if turn.is_some() && !streaming {
                        if limit {
                            let others: Vec<Provider> = chosen_accounts(&state)
                                .await
                                .into_iter()
                                .filter(|p| p.id != provider.id)
                                .collect();
                            let reason = (SwitchReason::Limit, "Usage limit reached".to_string());
                            if let Some((next, upstream, _)) =
                                connect_first(&state, others, &handshake, Some(reason)).await
                            {
                                let (mut tx, rx) = upstream.split();
                                let resent = match turn.clone() {
                                    Some(create) => tx.send(create).await.is_ok(),
                                    None => false,
                                };
                                if resent {
                                    log::info!(
                                        "[CodexWS] provider={} refused the turn for usage; sent it again on provider={}",
                                        provider.id,
                                        next.id
                                    );
                                    let _ = upstream_tx.send(Message::Close(None)).await;
                                    upstream_tx = tx;
                                    upstream_rx = rx;
                                    provider = next;
                                    held.clear();
                                    answered = false;
                                    continue 'relay;
                                }
                            }
                        } else if opens_turn(&kind) {
                            held.push(message);
                            continue;
                        }
                        streaming = true;
                        if let Some(log) = turn_log.as_mut() {
                            log.first_event_ms = Some(log.started.elapsed().as_millis() as u64);
                        }
                        for event in held.drain(..) {
                            if client_tx.send(event).await.is_err() {
                                break 'relay "client closed".to_string();
                            }
                        }
                    }
                    if kind == "response.completed" {
                        if let Some(log) = &turn_log {
                            log_completed(&state, &provider, log, &text);
                        }
                    }
                    if kind == "response.completed" && !answered {
                        answered = true;
                        super::codex_pool::save_login_of_serving_account(&state.db, &provider);
                    }
                    refused |= limit;
                    if ends_turn(&kind) {
                        turn = None;
                    }
                }
                let closing = matches!(message, Message::Close(_));
                if client_tx.send(message).await.is_err() || closing {
                    break "upstream closed".to_string();
                }
            }
            _ = check.tick() => {
                if turn.is_some() {
                    continue;
                }
                if refused {
                    break "account refused for usage".to_string();
                }
                let chosen = chosen_accounts(&state).await.into_iter().next().map(|p| p.id);
                if chosen.as_deref() != Some(provider.id.as_str()) {
                    break format!(
                        "pool now serves {}",
                        chosen.as_deref().unwrap_or("no ChatGPT account")
                    );
                }
            }
        }
    };

    log::info!(
        "[CodexWS] provider={}: websocket closed ({reason})",
        provider.id
    );
    let close = Message::Close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    }));
    let _ = client_tx.send(close.clone()).await;
    let _ = upstream_tx.send(close).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn turns_end_on_terminal_events_only() {
        assert_eq!(
            event_type(r#"{"type":"response.create","model":"m"}"#).as_deref(),
            Some("response.create")
        );
        assert_eq!(event_type("not json"), None);
        for kind in ["response.completed", "response.failed", "response.incomplete", "error"] {
            assert!(ends_turn(kind));
        }
        for kind in ["response.output_text.delta", "codex.rate_limits", "response.created"] {
            assert!(!ends_turn(kind));
        }
    }

    #[test]
    fn only_the_opening_of_a_turn_is_held_back() {
        assert!(opens_turn("response.created"));
        assert!(opens_turn("codex.rate_limits"));
        assert!(!opens_turn("response.output_item.added"));
        assert!(!opens_turn("response.output_text.delta"));
    }

    #[test]
    fn a_usage_limit_error_event_is_read_with_its_quota() {
        let text = serde_json::json!({
            "type": "error",
            "status": 429,
            "error": { "type": "usage_limit_reached", "resets_at": 1738888888 },
            "headers": { "x-codex-primary-used-percent": "100.0", "x-codex-primary-window-minutes": 15 }
        })
        .to_string();
        let (status, headers) = error_event(&text).expect("error event");
        assert_eq!(status, 429);
        assert_eq!(headers.get("x-codex-primary-used-percent").unwrap(), "100.0");
        assert_eq!(headers.get("x-codex-primary-window-minutes").unwrap(), "15");
        assert_eq!(
            error_event(r#"{"type":"error","status_code":400}"#).map(|e| e.0),
            Some(400)
        );
        assert!(error_event(r#"{"type":"response.created"}"#).is_none());
    }

    #[test]
    fn a_warm_up_turn_is_not_counted() {
        let handshake = ClientHandshake {
            headers: HeaderMap::new(),
            query: None,
        };
        let warmup = TurnLog::new(
            r#"{"type":"response.create","model":"gpt-x","generate":false}"#,
            &handshake,
        );
        assert!(warmup.warmup);
        assert_eq!(warmup.request_model, "gpt-x");
        let turn = TurnLog::new(r#"{"type":"response.create","model":"gpt-x"}"#, &handshake);
        assert!(!turn.warmup);
    }

    #[test]
    fn only_websocket_upgrades_are_relayed() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }
}
