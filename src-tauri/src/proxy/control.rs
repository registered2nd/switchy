//! Control routes on the proxy's own port, for a script or an agent to see and
//! act on the running app: which account each app is on, the order the pool
//! would serve them in and the quota behind it, and switching an account as
//! *Enable* on its card does. Reachable only where the proxy listens
//! (127.0.0.1 by default).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
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
        let current = AppType::from_str(app)
            .ok()
            .and_then(|t| {
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
        return fail(StatusCode::BAD_REQUEST, format!("unknown app {}", request.app));
    };
    let Some(app) = state.app_handle.clone() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "no app window".to_string());
    };
    let provider = request.provider.clone();
    let switched = tokio::task::spawn_blocking(move || {
        let app_state = app.state::<crate::store::AppState>();
        let result =
            crate::services::provider::ProviderService::switch(app_state.inner(), app_type, &provider);
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
