//! Keep-warm: one cheap request per pooled account, on a timer, off by
//! default.
//!
//! A subscription's session window — Anthropic's five hours, ChatGPT's — opens
//! on a real request and resets a fixed time later. An account nobody has used
//! has no window running, so the moment rotation moves onto it the window
//! starts from cold and the account is then the one carrying the session when
//! the next reset is furthest away. Keep-warm opens that window ahead of time:
//! for each pooled account whose window is not already running, one request
//! for a single token, sent the way a real request is sent — exit check first,
//! then the account's own stored login.
//!
//! Renewing the login is a byproduct rather than a second feature. Presenting
//! a stored login refreshes its access token when that is about to run out, so
//! an account that is kept warm is also kept signed in — as far as its refresh
//! token reaches, and no further. That deadline is anchored to the last
//! sign-in in the browser and renewing does not move it; when it passes, the
//! account needs a sign-in in the CLI itself and keep-warm cannot prevent it.
//!
//! What it costs is real quota with nobody present: a few tokens, a slice of
//! the session window and a touch of the weekly bucket, per account per
//! window. That is the whole reason it is off by default. An account whose
//! window is already running is skipped, and an account at its limit is
//! skipped, so the bill stays near one request per account per window.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::app_config::AppType;
use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::account_pool;
use crate::proxy::error::ProxyError;
use crate::proxy::{claude_pool, codex_pool};

/// How often the sweep looks at the accounts. The per-account gap is the
/// configured interval; this only bounds how late a warm-up can be.
const SWEEP_INTERVAL_SECS: u64 = 5 * 60;
/// Nothing is sent for this long after launch: the app is still restoring
/// proxy state and the user may be about to turn the switch back off.
const STARTUP_DELAY_SECS: u64 = 120;
/// Read and written by the sweep so a restart does not re-warm every account:
/// provider id → unix seconds of the last attempt.
const LAST_WARMED_KEY: &str = "keep_warm_last_warmed";
/// One token in, one token out.
const PROMPT: &str = "hi";
const MAX_TOKENS: u32 = 1;
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// Starts the sweep. Returns at once; the loop runs for the life of the app
/// and does nothing at all while the switch is off.
pub fn start(db: Arc<Database>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(STARTUP_DELAY_SECS)).await;
        loop {
            sweep(&db).await;
            tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS)).await;
        }
    });
}

/// Whether this account gets a request now.
fn is_due(
    last_warmed_at: Option<i64>,
    session_window_reset: Option<i64>,
    spent: bool,
    interval_minutes: u32,
    now: i64,
) -> bool {
    // At its limit: a request would only be refused.
    if spent {
        return false;
    }
    // A window still running is already warm; opening it again buys nothing.
    if session_window_reset.is_some_and(|reset| reset > now) {
        return false;
    }
    last_warmed_at.is_none_or(|last| now - last >= i64::from(interval_minutes) * 60)
}

fn load_last_warmed(db: &Database) -> HashMap<String, i64> {
    db.get_setting(LAST_WARMED_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn store_last_warmed(db: &Database, state: &HashMap<String, i64>) {
    let Ok(json) = serde_json::to_string(state) else {
        return;
    };
    if let Err(e) = db.set_setting(LAST_WARMED_KEY, &json) {
        log::warn!("[keep_warm] could not record the last warm-up time: {e}");
    }
}

/// The pooled accounts of one app, in queue order where there is one.
fn pooled_providers(db: &Database, app: &AppType) -> Vec<Provider> {
    let app_type = app.as_str();
    let Ok(all) = db.get_all_providers(app_type) else {
        return Vec::new();
    };
    let is_pooled = |p: &Provider| match app {
        AppType::Claude => claude_pool::is_oauth_provider(p),
        AppType::Codex => codex_pool::is_chatgpt_provider(p),
        _ => false,
    };
    let mut providers: Vec<Provider> = all.into_values().filter(is_pooled).collect();
    providers.sort_by(|a, b| a.sort_index.cmp(&b.sort_index).then(a.id.cmp(&b.id)));
    providers
}

async fn sweep(db: &Arc<Database>) {
    let config = db.get_account_pool_config().unwrap_or_default();
    if !config.keep_warm_enabled {
        return;
    }
    let models = db.get_stream_check_config().unwrap_or_default();
    let mut last_warmed = load_last_warmed(db);
    let mut wrote_any = false;

    for app in [AppType::Claude, AppType::Codex] {
        for provider in pooled_providers(db, &app) {
            let now = chrono::Utc::now().timestamp();
            if !is_due(
                last_warmed.get(&provider.id).copied(),
                account_pool::session_window_reset(&provider.id),
                account_pool::is_spent(&provider.id, None, config.threshold_percent, now),
                config.keep_warm_interval_minutes,
                now,
            ) {
                continue;
            }
            // Recorded before the result: a warm-up that fails must wait out
            // the interval like any other, or a broken account is retried
            // every sweep.
            last_warmed.insert(provider.id.clone(), now);
            wrote_any = true;

            let outcome = match app {
                AppType::Claude => warm_claude(db, &provider, &models.claude_model).await,
                _ => warm_codex(db, &provider, &models.codex_model).await,
            };
            match outcome {
                Ok(()) => log::info!(
                    "[keep_warm] opened the session window for provider={} ({})",
                    provider.id,
                    provider.name
                ),
                Err(e) => log::warn!(
                    "[keep_warm] provider={} ({}) was not warmed: {e}",
                    provider.id,
                    provider.name
                ),
            }
        }
    }

    if wrote_any {
        store_last_warmed(db, &last_warmed);
    }
}

/// One `/v1/messages` call for a single token, shaped like the ones Claude
/// Code sends through the proxy: the subscription beta, the CLI's identity,
/// and the selected account's captured login.
async fn warm_claude(
    db: &Arc<Database>,
    provider: &Provider,
    model: &str,
) -> Result<(), ProxyError> {
    account_pool::ensure_exit_allowed(db, provider, claude_pool::EXIT_TRACE_URL).await?;
    let token = claude_pool::access_token_for(provider, false).await?;

    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let response = crate::proxy::http_client::get_for_provider(proxy_config)
        .post(format!("{}/v1/messages", claude_pool::ANTHROPIC_BASE_URL))
        .bearer_auth(token)
        .header("anthropic-version", "2023-06-01")
        .header(
            "anthropic-beta",
            format!("claude-code-20250219,{}", claude_pool::OAUTH_BETA),
        )
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("user-agent", "claude-cli/2.1.2 (external, cli)")
        .header("x-app", "cli")
        .json(&json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "messages": [{ "role": "user", "content": PROMPT }],
        }))
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;

    let status = response.status();
    claude_pool::record_quota(&provider.id, Some(model), response.headers());
    fail_on_error_status(status.as_u16(), response.text().await.ok())
}

/// The same for a ChatGPT-login Codex account: one `/responses` call with the
/// account's bearer token and id, in the shape the Codex CLI itself sends.
async fn warm_codex(
    db: &Arc<Database>,
    provider: &Provider,
    model: &str,
) -> Result<(), ProxyError> {
    account_pool::ensure_exit_allowed(db, provider, codex_pool::EXIT_TRACE_URL).await?;
    let credentials = codex_pool::credentials_for(db, provider, false).await?;

    use crate::services::stream_check::StreamCheckService;
    let (model, effort) = chatgpt_warm_model(model, &read_codex_models_cache());
    let mut body = json!({
        "model": model,
        "input": [{ "role": "user", "content": PROMPT }],
        "stream": true,
        "store": false,
    });
    if let Some(effort) = effort {
        body["reasoning"] = json!({ "effort": effort });
    }

    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let mut request = crate::proxy::http_client::get_for_provider(proxy_config)
        .post(format!("{}/responses", codex_pool::CHATGPT_CODEX_BASE_URL))
        .bearer_auth(&credentials.access_token)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .header("accept-encoding", "identity")
        .header(
            "user-agent",
            format!(
                "codex_cli_rs/0.80.0 ({} 15.7.2; {}) Terminal",
                StreamCheckService::get_os_name(),
                StreamCheckService::get_arch_name()
            ),
        )
        .header("originator", "codex_cli_rs");
    if let Some(account_id) = credentials.account_id.as_deref() {
        request = request.header("chatgpt-account-id", account_id);
    }

    let response = request
        .json(&body)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| ProxyError::ForwardFailed(e.to_string()))?;

    let status = response.status();
    codex_pool::record_quota(&provider.id, response.headers());
    let body = response.text().await.ok();
    codex_pool::record_limit_refusal(&provider.id, status.as_u16(), body.as_deref());
    fail_on_error_status(status.as_u16(), body)
}

/// Codex's own cache of the models a ChatGPT login is offered, written by the
/// CLI from the ChatGPT backend; `Null` when Codex has not written one.
fn read_codex_models_cache() -> Value {
    let path = crate::codex_config::get_codex_config_dir().join("models_cache.json");
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

/// The model and effort a ChatGPT login is warmed with. The test model is an
/// API model by default, which a ChatGPT login refuses, so it is used only
/// when Codex's own list for ChatGPT logins has it; otherwise the model Codex
/// lists last, at its lightest effort. Reading the list rather than naming a
/// model keeps this current as OpenAI retires models.
fn chatgpt_warm_model(configured: &str, models_cache: &Value) -> (String, Option<String>) {
    use crate::services::stream_check::StreamCheckService;
    let (configured_model, configured_effort) =
        StreamCheckService::parse_model_with_effort(configured);

    let listed: Vec<&Value> = models_cache
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
                .collect()
        })
        .unwrap_or_default();
    let slug = |m: &Value| m.get("slug").and_then(Value::as_str).map(str::to_string);

    if listed
        .iter()
        .any(|m| slug(m).as_deref() == Some(configured_model.as_str()))
    {
        return (configured_model, configured_effort);
    }
    let last = listed
        .iter()
        .max_by_key(|m| m.get("priority").and_then(Value::as_i64).unwrap_or(0));
    match last.and_then(|m| slug(m).map(|s| (s, m))) {
        Some((model, entry)) => {
            let lightest = entry
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .and_then(|levels| levels.first())
                .and_then(|level| level.get("effort"))
                .and_then(Value::as_str)
                .map(str::to_string);
            (model, lightest)
        }
        // No list to go by: send the test model, and a refusal names it in
        // the log.
        None => (configured_model, configured_effort),
    }
}

/// The window opens on any answer upstream gives, including a refusal, and
/// the quota that came with it has already been recorded. The status is still
/// reported so a wrong model or a dead login shows up in the log.
fn fail_on_error_status(status: u16, body: Option<String>) -> Result<(), ProxyError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(ProxyError::UpstreamError {
        status,
        body: body.map(|b| b.chars().take(200).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CapturedClaudeAccountMeta, ProviderMeta};

    const NOW: i64 = 1_000_000;
    const HOUR: u32 = 60;

    fn claude_provider(id: &str, captured: bool, category: &str) -> Provider {
        let mut provider =
            Provider::with_id(id.to_string(), id.to_string(), json!({ "env": {} }), None);
        provider.category = Some(category.to_string());
        if captured {
            provider.meta = Some(ProviderMeta {
                captured_claude_account: Some(CapturedClaudeAccountMeta {
                    account_uuid: format!("uuid-{id}"),
                    email_address: format!("{id}@x.io"),
                    captured_at: 1,
                }),
                ..ProviderMeta::default()
            });
        }
        provider
    }

    #[test]
    fn only_official_claude_accounts_with_a_captured_login_are_warmed() {
        let db = Database::memory().unwrap();
        for (id, captured, category, sort) in [
            ("second", true, "official", 2),
            ("first", true, "official", 1),
            ("uncaptured", false, "official", 3),
            ("third-party", true, "third_party", 4),
        ] {
            let mut provider = claude_provider(id, captured, category);
            provider.sort_index = Some(sort);
            db.save_provider("claude", &provider).unwrap();
        }

        let warmed: Vec<String> = pooled_providers(&db, &AppType::Claude)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(warmed, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn an_api_key_codex_provider_is_not_warmed() {
        let db = Database::memory().unwrap();
        let provider = Provider::with_id(
            "key".to_string(),
            "key".to_string(),
            json!({ "auth": { "OPENAI_API_KEY": "sk-x" } }),
            None,
        );
        db.save_provider("codex", &provider).unwrap();
        assert!(pooled_providers(&db, &AppType::Codex).is_empty());
    }

    #[test]
    fn the_last_warmed_times_survive_a_round_trip_through_settings() {
        let db = Database::memory().unwrap();
        assert!(load_last_warmed(&db).is_empty());
        let mut state = HashMap::new();
        state.insert("p1".to_string(), 1_700_000_000_i64);
        store_last_warmed(&db, &state);
        assert_eq!(load_last_warmed(&db), state);
        // Anything unreadable in that row reads as "never warmed" rather than
        // stopping the sweep.
        db.set_setting(LAST_WARMED_KEY, "not json").unwrap();
        assert!(load_last_warmed(&db).is_empty());
    }

    #[test]
    fn an_account_never_warmed_with_no_window_running_is_due() {
        assert!(is_due(None, None, false, HOUR, NOW));
    }

    #[test]
    fn a_running_session_window_is_left_alone() {
        assert!(!is_due(None, Some(NOW + 60), false, HOUR, NOW));
        // A reset that has passed is a window that has lapsed.
        assert!(is_due(None, Some(NOW - 60), false, HOUR, NOW));
    }

    #[test]
    fn an_account_at_its_limit_is_not_warmed() {
        assert!(!is_due(None, None, true, HOUR, NOW));
    }

    #[test]
    fn the_interval_is_the_floor_between_two_attempts() {
        assert!(!is_due(Some(NOW - 1800), None, false, HOUR, NOW));
        assert!(is_due(Some(NOW - 3600), None, false, HOUR, NOW));
    }

    fn models_cache() -> Value {
        json!({ "models": [
            { "slug": "gpt-6-astra", "visibility": "list", "priority": 1,
              "supported_reasoning_levels": [{ "effort": "medium" }, { "effort": "high" }] },
            { "slug": "gpt-5.5", "visibility": "list", "priority": 12,
              "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "medium" }] },
            { "slug": "codex-auto-review", "visibility": "hide", "priority": 43,
              "supported_reasoning_levels": [{ "effort": "low" }] },
        ]})
    }

    #[test]
    fn a_chatgpt_login_is_warmed_with_a_model_codex_lists_for_it() {
        // The API test model is not offered to a ChatGPT login: the model
        // Codex lists last is used, at its lightest effort. Hidden models
        // are not candidates.
        assert_eq!(
            chatgpt_warm_model("gpt-5.1-codex@low", &models_cache()),
            ("gpt-5.5".to_string(), Some("low".to_string()))
        );
        // A test model that is on the list is kept, effort and all.
        assert_eq!(
            chatgpt_warm_model("gpt-6-astra@high", &models_cache()),
            ("gpt-6-astra".to_string(), Some("high".to_string()))
        );
        // No list: the test model goes out as configured.
        assert_eq!(
            chatgpt_warm_model("gpt-5.1-codex@low", &Value::Null),
            ("gpt-5.1-codex".to_string(), Some("low".to_string()))
        );
    }

    #[test]
    fn an_upstream_refusal_is_reported_but_a_success_is_not() {
        assert!(fail_on_error_status(200, None).is_ok());
        let err = fail_on_error_status(429, Some("x".repeat(500))).unwrap_err();
        match err {
            ProxyError::UpstreamError { status, body } => {
                assert_eq!(status, 429);
                assert_eq!(body.unwrap().chars().count(), 200);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
