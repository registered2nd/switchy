//! Official Claude accounts behind the proxy.
//!
//! Claude Code re-reads its credentials on every request, so the file swap
//! already moves an open session between accounts. What the proxy adds is
//! rotation without a swap: the failover queue becomes the pool, the login of
//! whichever Official provider is selected is presented on each request, and
//! the quota Anthropic reports on every response decides when to move on.
//!
//! Claude Code stays in its subscription mode. Takeover sets only
//! `ANTHROPIC_BASE_URL`; the login Claude Code sends is replaced here, and the
//! account id it writes into each request body is patched to match.
//!
//! The proxy is one more holder of each stored login, under the rules the
//! switch-away sync and the WSL reconciler already apply: it takes a newer
//! login for the same account from the live store before using a stored one,
//! and hands a login it renewed back to the live store when the recorded
//! owner says the live login is that account's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::provider::Provider;
use crate::proxy::account_pool::{self, QuotaWindow};
use crate::proxy::error::ProxyError;
use crate::services::claude_account::{self, paths, store};
use crate::services::credential_mirror::credential_health;

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// Asked, before a Claude login is used, where this machine is seen from.
pub const EXIT_TRACE_URL: &str = "https://api.anthropic.com/cdn-cgi/trace";
/// The beta Claude Code sends when it authenticates with its subscription.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

const TOKEN_ENDPOINT: &str = "https://platform.claude.com/v1/oauth/token";
/// Claude Code's own OAuth client: the one these logins were issued to.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Anthropic's edge refuses the token endpoint to clients it does not know;
/// this is the identity Claude Code's own refresh call carries.
const REFRESH_USER_AGENT: &str = "axios/1.13.6";

/// Refresh when the access token has less than this left.
const REFRESH_WINDOW_MS: i64 = 5 * 60 * 1000;
/// A 401 this soon after a refresh is about a request sent before it.
const FORCED_REFRESH_FLOOR_SECS: i64 = 60;

const OFFICIAL_CATEGORY: &str = "official";

// ── Recognising an Official provider ────────────────────────────────────────

/// True when the provider is an Official one whose login Switchy has captured.
pub fn is_oauth_provider(provider: &Provider) -> bool {
    provider.category.as_deref() == Some(OFFICIAL_CATEGORY) && account_uuid(provider).is_some()
}

/// The account the provider's captured login belongs to.
pub fn account_uuid(provider: &Provider) -> Option<&str> {
    provider
        .meta
        .as_ref()?
        .captured_claude_account
        .as_ref()
        .map(|c| c.account_uuid.as_str())
        .filter(|u| !u.is_empty())
}

// ── The stored login ────────────────────────────────────────────────────────

fn read_stored(provider_id: &str) -> Option<Value> {
    store::read_snapshot(&paths::snapshot_credentials_path(provider_id)).ok()
}

fn access_token(root: &Value) -> Option<String> {
    root.get("claudeAiOauth")?
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn refresh_token(root: &Value) -> Option<String> {
    root.get("claudeAiOauth")?
        .get("refreshToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Whether the live store holds this provider's account, by the recorded
/// owner marker — never by guessing from the identity file.
fn live_is_owned_by(provider: &Provider) -> bool {
    let Some(owner) = claude_account::read_live_owner() else {
        return false;
    };
    owner.provider_id == provider.id && Some(owner.account_uuid.as_str()) == account_uuid(provider)
}

/// If the live store holds this provider's login with a later expiry, Claude
/// Code has refreshed it since it was stored and the stored refresh token is
/// spent. Take the live one. Returns the login to use.
fn adopt_newer_live_login(provider: &Provider, stored: Value) -> Value {
    if !live_is_owned_by(provider) {
        return stored;
    }
    let Ok(Some(live_bytes)) = claude_account::read_live_credentials() else {
        return stored;
    };
    let Ok(live) = serde_json::from_slice::<Value>(&live_bytes) else {
        return stored;
    };
    let (live_alive, live_expires) = credential_health(&live);
    let (_, stored_expires) = credential_health(&stored);
    if !live_alive || live_expires <= stored_expires {
        return stored;
    }
    if let Err(e) =
        store::write_snapshot_atomic(&paths::snapshot_credentials_path(&provider.id), &live_bytes)
    {
        log::warn!(
            "[claude_pool] could not store the newer live login for provider={}: {e}",
            provider.id
        );
        return stored;
    }
    log::info!(
        "[claude_pool] provider={} took the newer login from the live store",
        provider.id
    );
    live
}

// ── Keeping a login usable ──────────────────────────────────────────────────

#[derive(Default)]
struct RefreshState {
    dead_refresh_token: Option<String>,
    last_refresh_at: i64,
}

static REFRESH_STATE: Lazy<Mutex<HashMap<String, RefreshState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static REFRESH_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn refresh_lock(provider_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = REFRESH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(provider_id.to_string()).or_default().clone()
}

/// The access token to present for `provider`, refreshed first when it is
/// about to run out. `force` refreshes regardless, in answer to a 401.
pub async fn access_token_for(provider: &Provider, force: bool) -> Result<String, ProxyError> {
    let lock = refresh_lock(&provider.id);
    let _guard = lock.lock().await;

    let stored = read_stored(&provider.id).ok_or_else(|| {
        ProxyError::AuthError(format!(
            "Claude provider {} has no captured login; capture it from the provider card",
            provider.name
        ))
    })?;
    let mut current = adopt_newer_live_login(provider, stored);

    let now_ms = chrono::Utc::now().timestamp_millis();
    let (_, expires_at) = credential_health(&current);
    let expiring = expires_at - now_ms < REFRESH_WINDOW_MS;
    let recently_refreshed = {
        let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.get(&provider.id).is_some_and(|s| {
            chrono::Utc::now().timestamp() - s.last_refresh_at < FORCED_REFRESH_FLOOR_SECS
        })
    };

    if expiring || (force && !recently_refreshed) {
        match refresh_login(provider, &current).await {
            Ok(renewed) => current = renewed,
            Err(e) if expiring => return Err(e),
            Err(e) => log::warn!("[claude_pool] forced refresh failed: {e}"),
        }
    }

    access_token(&current).ok_or_else(|| {
        ProxyError::AuthError(format!(
            "Claude provider {} has no usable login; sign in with `claude /login` while it is current and capture it again",
            provider.name
        ))
    })
}

async fn refresh_login(provider: &Provider, root: &Value) -> Result<Value, ProxyError> {
    let refresh_token = refresh_token(root).ok_or_else(|| {
        ProxyError::AuthError(format!(
            "Claude provider {} has no refresh token; sign in again while it is current",
            provider.name
        ))
    })?;

    {
        let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .get(&provider.id)
            .and_then(|s| s.dead_refresh_token.as_deref())
            == Some(refresh_token.as_str())
        {
            return Err(ProxyError::AuthError(format!(
                "Claude provider {} needs a new sign-in (its refresh token was rejected)",
                provider.name
            )));
        }
    }

    log::info!(
        "[claude_pool] refreshing login for provider={}",
        provider.id
    );
    let response = crate::proxy::http_client::get()
        .post(TOKEN_ENDPOINT)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/plain, */*")
        .header("User-Agent", REFRESH_USER_AGENT)
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ProxyError::AuthError(format!("Claude token refresh failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        if matches!(status.as_u16(), 400 | 401 | 403) {
            let mut state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
            state
                .entry(provider.id.clone())
                .or_default()
                .dead_refresh_token = Some(refresh_token);
        }
        return Err(ProxyError::AuthError(format!(
            "Claude token refresh for {} was refused ({status}): {}",
            provider.name,
            body.chars().take(200).collect::<String>()
        )));
    }

    let data: Value = response.json().await.map_err(|e| {
        ProxyError::AuthError(format!("Claude token refresh: unreadable reply: {e}"))
    })?;
    let renewed = apply_refresh_response(root, &data, chrono::Utc::now().timestamp_millis())
        .ok_or_else(|| {
            ProxyError::AuthError("Claude token refresh returned no access token".to_string())
        })?;

    let bytes = serde_json::to_vec_pretty(&renewed)
        .map_err(|e| ProxyError::Internal(format!("could not serialize refreshed login: {e}")))?;
    store::write_snapshot_atomic(&paths::snapshot_credentials_path(&provider.id), &bytes)
        .map_err(|e| ProxyError::Internal(format!("could not store refreshed login: {e}")))?;

    {
        let mut state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(provider.id.clone()).or_default();
        entry.dead_refresh_token = None;
        entry.last_refresh_at = chrono::Utc::now().timestamp();
    }

    propagate_refreshed_login(provider, &renewed);
    log::info!("[claude_pool] login refreshed for provider={}", provider.id);
    Ok(renewed)
}

/// The stored login with the token endpoint's reply folded in. A refresh
/// token comes back only when it was rotated; otherwise the old one stands.
/// `expires_in` is seconds; the stored `expiresAt` is milliseconds.
fn apply_refresh_response(root: &Value, data: &Value, now_ms: i64) -> Option<Value> {
    let access = data
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let mut result = root.clone();
    let oauth = result.get_mut("claudeAiOauth")?.as_object_mut()?;
    oauth.insert("accessToken".to_string(), json!(access));
    if let Some(r) = data
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        oauth.insert("refreshToken".to_string(), json!(r));
    }
    let expires_at = data
        .get("expires_at")
        .and_then(Value::as_i64)
        .map(|v| if v < 1_000_000_000_000 { v * 1000 } else { v })
        .or_else(|| {
            data.get("expires_in")
                .and_then(Value::as_i64)
                .filter(|s| *s > 0)
                .map(|s| now_ms + s * 1000)
        })
        .unwrap_or(now_ms + 3600 * 1000);
    oauth.insert("expiresAt".to_string(), json!(expires_at));
    Some(result)
}

/// A refresh spends the old refresh token everywhere it is held. When the
/// recorded owner says the live login is this account's, give the live store
/// the renewed one — only its `claudeAiOauth` block, so the machine's own
/// `mcpOAuth` keys stay — and move the marker's expiry with it. The WSL
/// reconciler carries it on from there.
fn propagate_refreshed_login(provider: &Provider, renewed: &Value) {
    if !live_is_owned_by(provider) {
        return;
    }
    let Some(oauth) = renewed.get("claudeAiOauth") else {
        return;
    };
    let mut live = match claude_account::read_live_credentials() {
        Ok(Some(bytes)) => serde_json::from_slice::<Value>(&bytes).unwrap_or(json!({})),
        _ => json!({}),
    };
    if !live.is_object() {
        live = json!({});
    }
    live["claudeAiOauth"] = oauth.clone();
    match serde_json::to_vec_pretty(&live) {
        Ok(bytes) => match claude_account::write_live_credentials(&bytes) {
            Ok(()) => {
                let (_, expires_at) = credential_health(renewed);
                if let Some(uuid) = account_uuid(provider) {
                    claude_account::write_live_owner(&provider.id, uuid, expires_at);
                }
                log::info!("[claude_pool] wrote the refreshed login through to the live store");
            }
            Err(e) => log::warn!("[claude_pool] could not update the live login: {e}"),
        },
        Err(e) => log::warn!("[claude_pool] could not serialize the live login: {e}"),
    }
}

// ── The request body ────────────────────────────────────────────────────────

/// Claude Code writes the signed-in account's id into `metadata.user_id`.
/// When another account's login is presented, the id is made to match, so the
/// body and the token agree. Both shapes Claude Code has used are handled: the
/// JSON string with an `account_uuid` field, and the older
/// `user_<hash>_account_<uuid>_session_<uuid>` form. Returns whether anything
/// changed.
pub fn patch_account_uuid(body: &mut Value, uuid: &str) -> bool {
    let Some(user_id) = body
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let patched = if let Ok(mut inner) = serde_json::from_str::<Value>(&user_id) {
        match inner.get("account_uuid").and_then(Value::as_str) {
            Some(existing) if existing == uuid => return false,
            Some(_) => {
                inner["account_uuid"] = json!(uuid);
                serde_json::to_string(&inner).unwrap_or(user_id)
            }
            None => return false,
        }
    } else if let Some(start) = user_id.find("_account_") {
        let value_start = start + "_account_".len();
        let value_end = user_id[value_start..]
            .find('_')
            .map(|i| value_start + i)
            .unwrap_or(user_id.len());
        if &user_id[value_start..value_end] == uuid {
            return false;
        }
        format!("{}{uuid}{}", &user_id[..value_start], &user_id[value_end..])
    } else {
        return false;
    };
    body["metadata"]["user_id"] = json!(patched);
    true
}

// ── Quota ───────────────────────────────────────────────────────────────────

/// Reads the `anthropic-ratelimit-unified-*` headers. Utilization is a 0–1
/// fraction; resets are unix seconds. Buckets are named by duration (`5h`,
/// `7d`); a bucket with a suffix (`7d_oi`) is model-scoped and does not spend
/// the account as a whole.
pub fn parse_quota_headers(headers: &http::HeaderMap) -> (Vec<QuotaWindow>, Option<i64>) {
    #[derive(Default)]
    struct Partial {
        utilization: Option<f64>,
        reset: Option<i64>,
        rejected: bool,
    }
    let mut buckets: HashMap<String, Partial> = HashMap::new();
    let mut overall_reset: Option<i64> = None;
    let mut overall_rejected = false;

    for (key, value) in headers {
        let Some(rest) = key.as_str().strip_prefix("anthropic-ratelimit-unified-") else {
            continue;
        };
        let Ok(value) = value.to_str() else { continue };
        let value = value.trim();
        match rest {
            "status" => overall_rejected = value == "rejected",
            "reset" => overall_reset = value.parse().ok(),
            _ => {
                let Some((bucket, field)) = rest.rsplit_once('-') else {
                    continue;
                };
                let entry = buckets.entry(bucket.to_string()).or_default();
                match field {
                    "utilization" => entry.utilization = value.parse().ok(),
                    "reset" => entry.reset = value.parse().ok(),
                    "status" => entry.rejected = value == "rejected",
                    _ => {}
                }
            }
        }
    }

    let now = chrono::Utc::now().timestamp();
    let mut limited_until: Option<i64> = None;
    let mut windows = Vec::new();
    for (bucket, p) in buckets {
        let (duration, scope) = bucket.split_once('_').unwrap_or((bucket.as_str(), ""));
        let minutes = match duration {
            "5h" => 300,
            "7d" => 10080,
            "1h" => 60,
            "1d" => 1440,
            _ => continue,
        };
        let Some(utilization) = p.utilization else {
            continue;
        };
        let account_wide = scope.is_empty();
        if account_wide && p.rejected {
            limited_until = Some(p.reset.filter(|r| *r > now).unwrap_or(now + 300));
        }
        windows.push(QuotaWindow {
            limit_name: (!account_wide).then(|| bucket.clone()),
            window_minutes: minutes,
            used_percent: utilization * 100.0,
            reset_at: p.reset.filter(|r| *r > 0),
        });
    }
    if overall_rejected && limited_until.is_none() {
        limited_until = Some(overall_reset.filter(|r| *r > now).unwrap_or(now + 300));
    }
    windows.sort_by(|a, b| {
        (a.limit_name.is_some(), &a.limit_name, a.window_minutes).cmp(&(
            b.limit_name.is_some(),
            &b.limit_name,
            b.window_minutes,
        ))
    });
    (windows, limited_until)
}

/// Records what a response said about the account that served it: its
/// windows, and a refusal when the account-wide status is `rejected`.
pub fn record_quota(provider_id: &str, headers: &http::HeaderMap) {
    let (windows, limited_until) = parse_quota_headers(headers);
    if !windows.is_empty() {
        account_pool::record_windows(provider_id, windows);
    }
    if let Some(until) = limited_until {
        account_pool::record_limited_until(provider_id, until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CapturedClaudeAccountMeta, ProviderMeta};

    fn official(uuid: Option<&str>) -> Provider {
        let mut meta = ProviderMeta::default();
        meta.captured_claude_account = uuid.map(|u| CapturedClaudeAccountMeta {
            account_uuid: u.to_string(),
            email_address: "a@x.io".to_string(),
            captured_at: 1,
        });
        let mut p = Provider::with_id("p".into(), "P".into(), json!({ "env": {} }), None);
        p.category = Some("official".into());
        p.meta = Some(meta);
        p
    }

    #[test]
    fn official_provider_with_captured_login_is_oauth() {
        assert!(is_oauth_provider(&official(Some("u-1"))));
        assert!(!is_oauth_provider(&official(None)));
        let mut third_party = official(Some("u-1"));
        third_party.category = Some("third_party".into());
        assert!(!is_oauth_provider(&third_party));
    }

    fn root(access: &str, refresh: &str, expires_at: i64) -> Value {
        json!({
            "claudeAiOauth": {
                "accessToken": access,
                "refreshToken": refresh,
                "expiresAt": expires_at,
                "scopes": ["user:inference"],
                "subscriptionType": "max"
            },
            "mcpOAuth": { "keep": true }
        })
    }

    #[test]
    fn refresh_reply_updates_token_and_expiry_and_keeps_the_rest() {
        let out = apply_refresh_response(
            &root("old", "RRR", 1),
            &json!({ "access_token": "new", "expires_in": 3600 }),
            1_000_000,
        )
        .unwrap();
        assert_eq!(out["claudeAiOauth"]["accessToken"], "new");
        assert_eq!(out["claudeAiOauth"]["refreshToken"], "RRR");
        assert_eq!(out["claudeAiOauth"]["expiresAt"], 1_000_000 + 3_600_000);
        assert_eq!(out["claudeAiOauth"]["subscriptionType"], "max");
        assert_eq!(out["mcpOAuth"]["keep"], true);
    }

    #[test]
    fn refresh_reply_takes_rotated_refresh_token_and_seconds_expiry() {
        let out = apply_refresh_response(
            &root("old", "RRR", 1),
            &json!({ "access_token": "new", "refresh_token": "SSS", "expires_at": 1_800_000_000 }),
            0,
        )
        .unwrap();
        assert_eq!(out["claudeAiOauth"]["refreshToken"], "SSS");
        assert_eq!(out["claudeAiOauth"]["expiresAt"], 1_800_000_000_000i64);
        assert!(apply_refresh_response(&root("o", "r", 1), &json!({}), 0).is_none());
    }

    #[test]
    fn account_uuid_is_patched_inside_the_json_user_id() {
        let mut body = json!({
            "model": "claude",
            "metadata": { "user_id": "{\"device_id\":\"d\",\"account_uuid\":\"aaaaaaaa-0000-0000-0000-000000000000\",\"session_id\":\"s\"}" }
        });
        assert!(patch_account_uuid(
            &mut body,
            "bbbbbbbb-0000-0000-0000-000000000000"
        ));
        let inner: Value =
            serde_json::from_str(body["metadata"]["user_id"].as_str().unwrap()).unwrap();
        assert_eq!(
            inner["account_uuid"],
            "bbbbbbbb-0000-0000-0000-000000000000"
        );
        assert_eq!(inner["device_id"], "d");
        assert!(!patch_account_uuid(
            &mut body,
            "bbbbbbbb-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn account_uuid_is_patched_in_the_legacy_user_id() {
        let mut body = json!({ "metadata": { "user_id": "user_abc_account_old-uuid_session_s1" } });
        assert!(patch_account_uuid(&mut body, "new-uuid"));
        assert_eq!(
            body["metadata"]["user_id"],
            "user_abc_account_new-uuid_session_s1"
        );
        let mut none = json!({ "metadata": { "user_id": "plain" } });
        assert!(!patch_account_uuid(&mut none, "x"));
        let mut missing = json!({ "model": "claude" });
        assert!(!patch_account_uuid(&mut missing, "x"));
    }

    fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut map = http::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                http::HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn unified_headers_become_windows_by_duration() {
        let (windows, limited) = parse_quota_headers(&headers(&[
            ("anthropic-ratelimit-unified-5h-utilization", "0.42"),
            ("anthropic-ratelimit-unified-5h-reset", "1900000000"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.9"),
            ("anthropic-ratelimit-unified-7d_oi-utilization", "1.1"),
            ("anthropic-ratelimit-unified-7d_oi-status", "rejected"),
            ("anthropic-ratelimit-unified-status", "allowed"),
        ]));
        assert_eq!(
            limited, None,
            "a model-scoped refusal does not spend the account"
        );
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].limit_name, None);
        assert_eq!(windows[0].window_minutes, 300);
        assert_eq!(windows[0].used_percent, 42.0);
        assert_eq!(windows[0].reset_at, Some(1_900_000_000));
        assert_eq!(windows[1].window_minutes, 10080);
        assert_eq!(windows[2].limit_name.as_deref(), Some("7d_oi"));
    }

    #[test]
    fn account_wide_refusal_spends_the_account_until_its_reset() {
        let (_, limited) = parse_quota_headers(&headers(&[
            ("anthropic-ratelimit-unified-5h-utilization", "1"),
            ("anthropic-ratelimit-unified-5h-status", "rejected"),
            ("anthropic-ratelimit-unified-5h-reset", "4102444800"),
            ("anthropic-ratelimit-unified-status", "rejected"),
        ]));
        assert_eq!(limited, Some(4_102_444_800));
    }
}
