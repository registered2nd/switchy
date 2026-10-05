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
//! Anything the relay cannot serve (a provider that is not a ChatGPT login, a
//! refused exit, a failed upstream handshake) is answered 426, which makes
//! Codex use the HTTP transport for the rest of its session.

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Role};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::server::ProxyState;
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

/// The ChatGPT account the pool would serve Codex from now, if it is one.
async fn chosen_account(state: &ProxyState) -> Option<Provider> {
    let providers = state
        .provider_router
        .select_providers("codex", None)
        .await
        .ok()?;
    providers
        .into_iter()
        .next()
        .filter(super::codex_pool::is_chatgpt_provider)
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
    let Some(provider) = chosen_account(&state).await else {
        return fall_back_to_http();
    };

    // The request body is not Sync, so nothing borrows the request across an await.
    let client_headers = request.headers().clone();
    let query = request.uri().query().map(str::to_string);
    let (upstream, upstream_headers) =
        match connect_upstream(&state, &provider, &client_headers, query.as_deref()).await {
        Ok(connected) => connected,
        Err(e) => {
            log::warn!(
                "[CodexWS] provider={}: websocket not relayed ({e}); Codex falls back to HTTP",
                provider.id
            );
            return fall_back_to_http();
        }
    };
    super::codex_pool::record_quota(&provider.id, &upstream_headers);
    log::info!(
        "[CodexWS] relaying a Codex websocket through provider={}",
        provider.id
    );

    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let client =
                    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)
                        .await;
                relay(state, provider, client, upstream).await;
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

type Upstream = WebSocketStream<reqwest::Upgraded>;

/// Opens the WebSocket to the ChatGPT backend with `provider`'s login, after
/// the exit check. Returns the upstream handshake's response headers.
async fn connect_upstream(
    state: &ProxyState,
    provider: &Provider,
    client_headers: &HeaderMap,
    query: Option<&str>,
) -> Result<(Upstream, HeaderMap), String> {
    super::account_pool::ensure_exit_allowed(
        &state.db,
        provider,
        super::codex_pool::EXIT_TRACE_URL,
    )
    .await
    .map_err(|e| e.to_string())?;
    let credentials = super::codex_pool::credentials_for(&state.db, provider, false)
        .await
        .map_err(|e| e.to_string())?;

    let mut url = format!("{}/responses", super::codex_pool::CHATGPT_CODEX_BASE_URL);
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let client = super::http_client::get_http1_for_provider(proxy_config)?;

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
    for (name, value) in client_headers {
        if !NOT_FORWARDED.contains(&name.as_str()) {
            upstream = upstream.header(name, value);
        }
    }

    let response = upstream
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    if status != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        let body = response.text().await.unwrap_or_default();
        super::codex_pool::record_limit_refusal(&provider.id, status.as_u16(), Some(&body));
        return Err(format!(
            "upstream answered {status}: {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    let headers = response.headers().clone();
    let upgraded = response.upgrade().await.map_err(|e| e.to_string())?;
    let socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;
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

/// Moves frames both ways until either side closes, or until the pool would
/// serve another account and no turn is in flight.
async fn relay(
    state: ProxyState,
    provider: Provider,
    client: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    upstream: Upstream,
) {
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let mut in_flight = false;
    let mut answered = false;
    let mut refused = false;
    let mut check = tokio::time::interval(ACCOUNT_CHECK);
    check.tick().await;

    let reason = loop {
        tokio::select! {
            message = client_rx.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => break format!("client error: {e}"),
                    None => break "client closed".to_string(),
                };
                if let Message::Text(text) = &message {
                    if event_type(text.as_str()).as_deref() == Some("response.create") {
                        in_flight = true;
                    }
                }
                let closing = matches!(message, Message::Close(_));
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
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
                if let Message::Text(text) = &message {
                    if let Some(kind) = event_type(text.as_str()) {
                        if kind == "response.completed" && !answered {
                            answered = true;
                            super::codex_pool::save_login_of_serving_account(&state.db, &provider);
                        }
                        if kind == "error" {
                            let status = serde_json::from_str::<Value>(text.as_str())
                                .ok()
                                .and_then(|v| v.get("status").and_then(Value::as_u64));
                            if let Some(status) = status {
                                super::codex_pool::record_limit_refusal(
                                    &provider.id,
                                    status as u16,
                                    Some(text.as_str()),
                                );
                                refused |= status == 429;
                            }
                        }
                        if ends_turn(&kind) {
                            in_flight = false;
                        }
                    }
                }
                let closing = matches!(message, Message::Close(_));
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
                if client_tx.send(message).await.is_err() || closing {
                    break "upstream closed".to_string();
                }
            }
            _ = check.tick() => {
                if in_flight {
                    continue;
                }
                if refused {
                    break "account refused for usage".to_string();
                }
                let chosen = chosen_account(&state).await.map(|p| p.id);
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
    fn only_websocket_upgrades_are_relayed() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }
}
