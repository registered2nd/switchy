//! Control routes on the proxy's own port, for a script or an agent to see and
//! act on the running app: which account each app is on, the order the pool
//! would serve them in and the quota behind it, switching an account as
//! *Enable* on its card does, and calling any command the window calls.
//! Reachable only where the proxy listens (127.0.0.1 by default); the port is
//! served from launch, with the proxy on or off.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use tauri::{Emitter, Manager};

use super::server::ProxyState;
use crate::app_config::AppType;

const POOLED_APPS: [&str; 2] = ["claude", "codex"];

/// `GET /switchy/accounts`: per pooled app, the current account, the account
/// held after a pick by hand, and the switching order as the pool would serve
/// a request now, each account with its last quota reading and whether it is
/// at its limit.
pub async fn accounts(State(state): State<ProxyState>) -> Json<Value> {
    let pool = state.db.get_account_pool_config().unwrap_or_default();
    let quotas = super::account_pool::quota_snapshot();
    let now = chrono::Utc::now().timestamp();
    let mut apps = serde_json::Map::new();
    for app in POOLED_APPS {
        let current = AppType::from_str(app).ok().and_then(|t| {
            crate::settings::get_effective_current_provider(&state.db, &t)
                .ok()
                .flatten()
        });
        let order = match state.provider_router.select_providers(app, None).await {
            Ok(providers) => providers
                .into_iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "name": p.name,
                        "atLimit": super::account_pool::is_spent(
                            &p.id,
                            None,
                            pool.threshold_percent,
                            now,
                        ),
                        "quota": quotas.get(&p.id),
                    })
                })
                .collect(),
            Err(e) => vec![json!({ "error": e.to_string() })],
        };
        apps.insert(
            app.to_string(),
            json!({
                "current": current,
                "heldByHand": super::manual_hold::held(app),
                "locked": super::manual_hold::locked(app),
                "order": order,
            }),
        );
    }
    Json(json!({
        "rotation": { "enabled": pool.enabled, "thresholdPercent": pool.threshold_percent },
        "apps": apps,
    }))
}

#[derive(Deserialize)]
pub struct SwitchRequest {
    app: String,
    provider: String,
}

/// `POST /switchy/switch` with `{"app": "codex", "provider": "<id>"}`: makes
/// the account current as *Enable* on its card does, holding it for the
/// manual-pick period, and refreshes the window and the tray.
pub async fn switch(
    State(state): State<ProxyState>,
    Json(request): Json<SwitchRequest>,
) -> (StatusCode, Json<Value>) {
    let fail = |status: StatusCode, message: String| (status, Json(json!({ "error": message })));
    let Ok(app_type) = AppType::from_str(&request.app) else {
        return fail(
            StatusCode::BAD_REQUEST,
            format!("unknown app {}", request.app),
        );
    };
    let Some(app) = state.app_handle.clone() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "no app window".to_string());
    };
    let provider = request.provider.clone();
    let switched = tokio::task::spawn_blocking(move || {
        let app_state = app.state::<crate::store::AppState>();
        let result = crate::services::provider::ProviderService::switch(
            app_state.inner(),
            app_type,
            &provider,
        );
        if result.is_ok() {
            if let Ok(menu) = crate::tray::create_tray_menu(&app, app_state.inner()) {
                if let Some(tray) = app.tray_by_id("main") {
                    let _ = tray.set_menu(Some(menu));
                }
            }
            let _ = app.emit(
                "provider-switched",
                json!({ "appType": request.app, "providerId": provider }),
            );
        }
        result
    })
    .await;
    match switched {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(json!({ "switched": request.provider, "warnings": result.warnings })),
        ),
        Ok(Err(e)) => fail(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// The address a request to the proxy's port came from.
#[derive(Clone, Copy)]
pub struct Peer(pub std::net::SocketAddr);

/// Every command the window can call and the arguments each takes, built from
/// the source by `build.rs`.
const COMMANDS: &str = include_str!(concat!(env!("OUT_DIR"), "/commands.json"));

/// How long a command may run before the caller is answered with a timeout.
const INVOKE_TIMEOUT_SECS: u64 = 300;

/// `GET /switchy/commands`: the commands `POST /switchy/invoke` accepts, each
/// with its argument names (camelCase) and Rust types.
pub async fn commands(
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(refused) = local_caller(peer, &headers) {
        return refused;
    }
    let list: Value = serde_json::from_str(COMMANDS).unwrap_or_else(|_| json!([]));
    (StatusCode::OK, Json(json!({ "commands": list })))
}

#[derive(Deserialize)]
pub struct InvokeRequest {
    command: String,
    #[serde(default)]
    args: Value,
}

/// `POST /switchy/invoke` with `{"command": "<name>", "args": {...}}`: runs the
/// command exactly as the window would, through the window's own command
/// handler, and answers with its result. Everything the window can do is
/// reachable this way. The window is refreshed after a command that is not a
/// read.
pub async fn invoke(
    State(state): State<ProxyState>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    Json(request): Json<InvokeRequest>,
) -> (StatusCode, Json<Value>) {
    use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};

    if let Err(refused) = local_caller(peer, &headers) {
        return refused;
    }
    let fail = |status: StatusCode, message: String| (status, Json(json!({ "error": message })));
    let Some(app) = state.app_handle.clone() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "no app window".to_string());
    };
    let Some(window) = app.get_webview_window("main") else {
        // Lightweight mode closed the window: the commands that bring it back
        // or quit run without it; the rest need it.
        return match request.command.as_str() {
            "exit_lightweight_mode" | "show_main_window" => {
                match crate::lightweight::exit_lightweight_mode(&app) {
                    Ok(()) => (StatusCode::OK, Json(json!({ "result": null }))),
                    Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                }
            }
            "is_lightweight_mode" => (StatusCode::OK, Json(json!({ "result": true }))),
            "quit_app" => {
                crate::commands::quit_app(app.clone());
                (StatusCode::OK, Json(json!({ "result": null })))
            }
            _ => fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "the main window is closed (lightweight mode): run exit_lightweight_mode first"
                    .to_string(),
            ),
        };
    };
    let url = match window.url() {
        Ok(url) => url,
        Err(e) => return fail(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
    };
    let args = if request.args.is_null() {
        json!({})
    } else {
        request.args
    };
    let message = tauri::webview::InvokeRequest {
        cmd: request.command.clone(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url,
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: app.invoke_key().to_string(),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let webview: tauri::Webview = window.as_ref().clone();
    if let Err(e) = app.run_on_main_thread(move || {
        webview.on_message(
            message,
            Box::new(move |_, _, response, _, _| {
                let _ = tx.send(response);
            }),
        )
    }) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let response =
        match tokio::time::timeout(std::time::Duration::from_secs(INVOKE_TIMEOUT_SECS), rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                return fail(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the command gave no answer".to_string(),
                )
            }
            Err(_) => {
                return fail(
                    StatusCode::GATEWAY_TIMEOUT,
                    format!(
                        "{} did not finish in {INVOKE_TIMEOUT_SECS} seconds",
                        request.command
                    ),
                )
            }
        };
    match response {
        InvokeResponse::Ok(body) => {
            if !is_read(&request.command) {
                crate::tray::refresh_tray_menu(&app);
                let _ = app.emit("agent-invoked", json!({ "command": request.command }));
            }
            let result = match body {
                InvokeResponseBody::Json(text) => {
                    serde_json::from_str(&text).unwrap_or(Value::String(text))
                }
                InvokeResponseBody::Raw(bytes) => json!(bytes),
            };
            (StatusCode::OK, Json(json!({ "result": result })))
        }
        InvokeResponse::Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error.0 }))),
    }
}

/// Whether a command only reads, so the window need not refresh after it.
fn is_read(command: &str) -> bool {
    ["get_", "is_", "list_", "read_", "check_", "detect_"]
        .iter()
        .any(|prefix| command.starts_with(prefix))
}

/// The command routes hand out keys and change everything, so they answer
/// only a caller on this machine that is not a web page: a browser always
/// sends `Origin` on these requests, and the router's open CORS would
/// otherwise let any site read the answer.
fn local_caller(peer: Peer, headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    if !peer.0.ip().is_loopback() || headers.contains_key(axum::http::header::ORIGIN) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "only a local, non-browser caller can use this route" })),
        ));
    }
    Ok(())
}
