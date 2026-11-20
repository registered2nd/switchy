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
//! owner says the live login is that account's. WSL's Claude Code is held to
//! the same rules by account: a newer login of the account WSL is signed in
//! to is taken from there before renewing, and a renewed one is written there.
//! A login one Claude Code install holds is renewed by that install: the proxy
//! renews it only once it has run out without a renewal, so the two never post
//! the same single-use refresh token together. A login both installs hold,
//! Windows and WSL, is renewed here before either install's window opens and
//! written to both, since the two installs would otherwise post it together.

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

pub(crate) const TOKEN_ENDPOINT: &str = "https://platform.claude.com/v1/oauth/token";
/// Claude Code's own OAuth client: the one these logins were issued to.
pub(crate) const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Anthropic's edge refuses the token endpoint to clients it does not know;
/// this is the identity Claude Code's own refresh call carries.
pub(crate) const REFRESH_USER_AGENT: &str = "axios/1.13.6";

/// Refresh a login only the proxy holds when its access token has less than
/// this left.
const REFRESH_WINDOW_MS: i64 = 5 * 60 * 1000;
/// A login a Claude Code install also holds (Windows signed in to it, or WSL)
/// is renewed by that install once less than five minutes are left, the same
/// window as `REFRESH_WINDOW_MS`. Posting the same single-use refresh token
/// alongside it loses the login for both, so the proxy keeps presenting the
/// still-valid access token, takes the install's renewal, and renews such a
/// login itself only once it has run out.
const HELD_REFRESH_WINDOW_MS: i64 = 0;
/// A login both Claude Code installs hold, Windows and WSL, would be renewed
/// by each of them in that same window, posting the same refresh token
/// together. It is renewed here while it has less than this left and at least
/// `REFRESH_WINDOW_MS`: before either install's window opens.
const SHARED_REFRESH_WINDOW_MS: i64 = 10 * 60 * 1000;
/// How long a renewal of a login a Claude Code install holds waits for that
/// install's own renewal to appear before making one here.
const HELD_RENEWAL_WAIT_MS: u64 = 5_000;
const HELD_RENEWAL_POLL_MS: u64 = 250;
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
    note_renewed(&provider.id);
    log::info!(
        "[claude_pool] provider={} took the newer login from the live store",
        provider.id
    );
    live
}

/// The WSL install's credentials file, when WSL is signed in to `provider`'s
/// account. The account is read from WSL's `.claude.json`, so a switch that
/// could not reach WSL and left it on an earlier account is still matched.
fn mirror_credentials_of(provider: &Provider) -> Option<std::path::PathBuf> {
    let uuid = account_uuid(provider)?;
    let dir = crate::settings::get_claude_mirror_override_dir()?;
    let signed_in = crate::services::credential_mirror::read_account_uuid(
        &paths::mirror_claude_config_path(&dir),
    )?;
    (signed_in == uuid).then(|| dir.join(".credentials.json"))
}

fn read_login_at(path: &std::path::Path) -> Option<Value> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
}

/// The login in the Windows (or macOS) live store, whoever it belongs to.
fn read_live_login() -> Option<Value> {
    let bytes = claude_account::read_live_credentials().ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// WSL's credentials file, whichever account WSL is signed in to.
fn wsl_credentials_path() -> Option<std::path::PathBuf> {
    crate::settings::get_claude_mirror_override_dir().map(|dir| dir.join(".credentials.json"))
}

/// If WSL's Claude Code is signed in to this provider's account and has
/// renewed the login since it was stored, the stored refresh token is spent.
/// Take WSL's login, keeping this machine's own keys. Returns the login to use
/// and whether it came from WSL.
fn adopt_newer_mirror_login(provider: &Provider, current: Value) -> (Value, bool) {
    let Some(path) = mirror_credentials_of(provider) else {
        return (current, false);
    };
    let Some(mirror) = read_login_at(&path) else {
        return (current, false);
    };
    let (mirror_alive, mirror_expires) = credential_health(&mirror);
    let (_, current_expires) = credential_health(&current);
    if !mirror_alive
        || mirror_expires <= current_expires
        || refresh_token(&mirror) == refresh_token(&current)
    {
        return (current, false);
    }
    let Some(oauth) = mirror.get("claudeAiOauth") else {
        return (current, false);
    };
    let mut adopted = current.clone();
    adopted["claudeAiOauth"] = oauth.clone();
    let stored = serde_json::to_vec_pretty(&adopted)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            store::write_snapshot_atomic(&paths::snapshot_credentials_path(&provider.id), &bytes)
                .map_err(|e| e.to_string())
        });
    if let Err(e) = stored {
        log::warn!(
            "[claude_pool] could not store the newer WSL login for provider={}: {e}",
            provider.id
        );
        return (current, false);
    }
    note_renewed(&provider.id);
    log::info!(
        "[claude_pool] provider={} took the newer login from WSL",
        provider.id
    );
    (adopted, true)
}

// ── Keeping a login usable ──────────────────────────────────────────────────

#[derive(Default)]
struct RefreshState {
    dead_refresh_token: Option<String>,
    last_refresh_at: i64,
}

static REFRESH_STATE: Lazy<Mutex<HashMap<String, RefreshState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Records that `provider_id`'s stored login was just replaced by a renewed
/// one, made here or taken from a Claude Code install.
fn note_renewed(provider_id: &str) {
    REFRESH_STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(provider_id.to_string())
        .or_default()
        .last_refresh_at = chrono::Utc::now().timestamp();
}

/// A 401 this soon after the stored login was renewed is about a request sent
/// with the login it replaced; the renewal already answers it.
fn renewed_recently(provider_id: &str) -> bool {
    REFRESH_STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider_id)
        .is_some_and(|s| {
            chrono::Utc::now().timestamp() - s.last_refresh_at < FORCED_REFRESH_FLOOR_SECS
        })
}

/// Whether Anthropic has refused `provider`'s captured refresh token, so the
/// account stays unusable until it is signed in and captured again.
pub fn needs_sign_in(provider: &Provider) -> bool {
    let Some(current) =
        read_stored(&provider.id).map(|stored| adopt_newer_live_login(provider, stored))
    else {
        return false;
    };
    if !refresh_token_refused(provider, &current) {
        return false;
    }
    // WSL may have renewed the account since; its login replaces the refused one.
    let (current, _) = adopt_newer_mirror_login(provider, current);
    refresh_token_refused(provider, &current)
}

/// Forgets that `provider_id`'s refresh token was refused, once a new sign-in
/// is stored with it.
pub(crate) fn clear_refused(provider_id: &str) {
    if let Some(state) = REFRESH_STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(provider_id)
    {
        state.dead_refresh_token = None;
    }
}

/// Whether Anthropic refused `refresh_token` for any provider, so a copy of
/// that login anywhere is spent.
pub(crate) fn refresh_token_was_refused(refresh_token: &str) -> bool {
    !refresh_token.is_empty()
        && REFRESH_STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|s| s.dead_refresh_token.as_deref() == Some(refresh_token))
}

fn refresh_token_refused(provider: &Provider, login: &Value) -> bool {
    let Some(refresh_token) = refresh_token(login) else {
        return false;
    };
    let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    state
        .get(&provider.id)
        .and_then(|s| s.dead_refresh_token.as_deref())
        == Some(refresh_token.as_str())
}

static REFRESH_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn refresh_lock(provider_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = REFRESH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(provider_id.to_string()).or_default().clone()
}

/// Which Claude Code installs hold a login besides the proxy. Each install
/// renews the login it holds on its own schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holders {
    /// Neither: the proxy is its only renewer.
    ProxyOnly,
    /// Windows or WSL, which renews it.
    OneInstall,
    /// Windows and WSL, which would each renew it in the same five minutes.
    BothInstalls,
}

/// An install holds `login` when it is signed in to `provider` (Windows by the
/// recorded owner, WSL by account) or its credentials carry the same refresh
/// token, which a capture or a copy between installs leaves without a marker.
fn holders(provider: &Provider, login: &Value) -> Holders {
    let token = refresh_token(login);
    let carries =
        |other: Option<Value>| token.is_some() && other.as_ref().and_then(refresh_token) == token;
    let windows = live_is_owned_by(provider) || carries(read_live_login());
    let wsl = mirror_credentials_of(provider).is_some()
        || carries(wsl_credentials_path().and_then(|path| read_login_at(&path)));
    match (windows, wsl) {
        (true, true) => Holders::BothInstalls,
        (false, false) => Holders::ProxyOnly,
        _ => Holders::OneInstall,
    }
}

/// The live store carrying `login`'s refresh token holds this card's login,
/// whatever the marker says: a capture or a copy from WSL leaves it naming
/// another card. Record the pairing, so Claude Code's renewal of the login is
/// taken from the live store like any other instead of the proxy later
/// posting the token that renewal spent.
fn record_live_owner_if_carried(provider: &Provider, login: &Value) {
    if live_is_owned_by(provider) {
        return;
    }
    let (Some(token), Some(uuid)) = (refresh_token(login), account_uuid(provider)) else {
        return;
    };
    let Some(live) = read_live_login() else {
        return;
    };
    if refresh_token(&live).as_deref() == Some(token.as_str()) {
        let (_, expires_at) = credential_health(&live);
        claude_account::write_live_owner(&provider.id, uuid, expires_at);
        log::info!(
            "[claude_pool] the live store holds provider={}'s login; recorded it as the owner",
            provider.id
        );
    }
}

/// What to do with a login about to be presented.
#[derive(Debug, PartialEq, Eq)]
enum Renewal {
    /// Present it as it is.
    Present,
    /// Renew it here.
    Renew,
    /// Give the Claude Code install that holds it a moment to renew it, and
    /// renew it here only if that renewal does not appear.
    AwaitHolder,
}

/// Whether a login both installs hold is due to be renewed here: five to ten
/// minutes left, before either install's window opens.
pub(crate) fn renews_ahead_of_installs(remaining_ms: i64) -> bool {
    (REFRESH_WINDOW_MS..SHARED_REFRESH_WINDOW_MS).contains(&remaining_ms)
}

/// `remaining_ms` is what the access token has left; `forced` is a 401 on it.
///
/// Each login has one renewer at a time. The proxy renews a login only it
/// holds. One install holding it renews it, and the proxy steps in only after
/// it has run out unrenewed. Two installs holding it would renew it together,
/// so the proxy renews it before their window and, if that fails, keeps out of
/// their window rather than post the token a third time.
fn renewal(remaining_ms: i64, holders: Holders, forced: bool) -> Renewal {
    match holders {
        Holders::ProxyOnly if forced || remaining_ms < REFRESH_WINDOW_MS => Renewal::Renew,
        Holders::ProxyOnly => Renewal::Present,
        _ if forced || remaining_ms < HELD_REFRESH_WINDOW_MS => Renewal::AwaitHolder,
        Holders::BothInstalls if renews_ahead_of_installs(remaining_ms) => Renewal::Renew,
        _ => Renewal::Present,
    }
}

/// A login newer than `current` from the live store or WSL, if one appears
/// within `HELD_RENEWAL_WAIT_MS`: the renewal of the Claude Code install that
/// holds it.
async fn await_holder_renewal(provider: &Provider, current: &Value) -> Option<Value> {
    let (_, before) = credential_health(current);
    let polls = HELD_RENEWAL_WAIT_MS / HELD_RENEWAL_POLL_MS;
    for _ in 0..polls {
        tokio::time::sleep(std::time::Duration::from_millis(HELD_RENEWAL_POLL_MS)).await;
        let live = adopt_newer_live_login(provider, current.clone());
        let (candidate, _) = adopt_newer_mirror_login(provider, live);
        if credential_health(&candidate).1 > before {
            return Some(candidate);
        }
    }
    None
}

/// The access token to present for `provider`, refreshed first when it is
/// about to run out. `force` refreshes regardless, in answer to a 401.
///
/// A login one Claude Code install also holds is left to that install to
/// renew: the proxy presents it until it has run out, and before renewing it
/// (on expiry or a 401) waits briefly for the install's renewal and takes it.
/// A login both installs hold is renewed here before their window, then
/// treated the same way. A 401 answered by a login taken from the live store
/// or WSL in this call is already answered: that newer login is presented
/// without renewing it. Every caller (requests, keep-warm, usage reads) comes
/// through here, under one lock per card.
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
    if expires_at - now_ms < SHARED_REFRESH_WINDOW_MS || (force && !renewed_recently(&provider.id))
    {
        // A login WSL renewed already is the renewal; refreshing the stored
        // token would be refused, and would spend WSL's if it were not.
        let (adopted, _) = adopt_newer_mirror_login(provider, current);
        current = adopted;
        let (_, expires_at) = credential_health(&current);
        let forced = force && !renewed_recently(&provider.id);
        let remaining = expires_at - now_ms;
        record_live_owner_if_carried(provider, &current);
        let mut action = renewal(remaining, holders(provider, &current), forced);
        if action == Renewal::AwaitHolder && refresh_token_refused(provider, &current) {
            // Anthropic refused this login: no install will renew it, and
            // refresh_login answers at once that it needs a new sign-in.
            action = Renewal::Renew;
        }
        match action {
            Renewal::Present => {}
            action => {
                let mut renewed_elsewhere = false;
                if action == Renewal::AwaitHolder {
                    if let Some(renewed) = await_holder_renewal(provider, &current).await {
                        current = renewed;
                        renewed_elsewhere = true;
                    }
                }
                if !renewed_elsewhere {
                    let expiring = remaining < REFRESH_WINDOW_MS;
                    match refresh_login(provider, &current).await {
                        Ok(renewed) => current = renewed,
                        Err(e) if expiring => return Err(e),
                        Err(e) => log::warn!(
                            "[claude_pool] renewal of provider={} failed; presenting its current login: {e}",
                            provider.id
                        ),
                    }
                }
            }
        }
    }

    access_token(&current).ok_or_else(|| {
        ProxyError::AuthError(format!(
            "Claude provider {} has no usable login; sign in with `claude /login` while it is current and capture it again",
            provider.name
        ))
    })
}

/// The renewal of a login both installs hold, made ahead of their window. The
/// credential reconciler calls this when Windows and WSL hold `refresh_token`
/// with five to ten minutes left, so the renewal does not wait for a request
/// to pass through the proxy. It is made through the card whose stored login
/// that is, under the card's lock and rules; a login no card holds is left to
/// the installs, since Switchy renews captured logins only.
pub(crate) async fn renew_shared_login(db: &crate::database::Database, refresh_token: &str) {
    if refresh_token_was_refused(refresh_token) {
        return;
    }
    let Ok(providers) = db.get_all_providers("claude") else {
        return;
    };
    let card = providers.values().filter(|p| is_oauth_provider(p)).find(|p| {
        read_stored(&p.id)
            .map(|stored| adopt_newer_live_login(p, stored))
            .and_then(|login| self::refresh_token(&login))
            .as_deref()
            == Some(refresh_token)
    });
    let Some(card) = card else {
        static NOTED: Mutex<Option<String>> = Mutex::new(None);
        let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
        if noted.as_deref() != Some(refresh_token) {
            *noted = Some(refresh_token.to_string());
            log::info!(
                "[claude_pool] Windows and WSL hold a login no card has captured; each install renews it itself"
            );
        }
        return;
    };
    if let Err(e) = access_token_for(card, false).await {
        log::warn!(
            "[claude_pool] renewal ahead of both installs failed for provider={}: {e}",
            card.id
        );
    }
}

/// Stores the login WSL holds of `provider`'s account when WSL has renewed it
/// since it was stored. A switch calls this before it overwrites WSL's
/// credentials: that renewal is then the only live copy of the login.
pub(crate) fn take_newer_wsl_login(provider: &Provider) {
    if let Some(stored) = read_stored(&provider.id) {
        let _ = adopt_newer_mirror_login(provider, stored);
    }
}

/// A login renewal the token endpoint answered with an error. Only a rejection
/// of the token is a login problem; a server error or rate limit is the
/// service being unreachable, reported like a dropped connection so the app
/// retries instead of asking the user to sign in again.
pub(crate) fn refresh_refusal(status: u16, message: String) -> ProxyError {
    if matches!(status, 400 | 401 | 403) {
        ProxyError::AuthError(message)
    } else {
        ProxyError::ForwardFailed(message)
    }
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
    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let response = crate::proxy::http_client::get_for_provider(proxy_config)
        .map_err(ProxyError::ForwardFailed)?
        .post(token_endpoint())
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
        .map_err(|e| ProxyError::ForwardFailed(format!("Claude token refresh failed: {e}")))?;

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
        let message = format!(
            "Claude token refresh for {} was refused ({status}): {}",
            provider.name,
            body.chars().take(200).collect::<String>()
        );
        return Err(refresh_refusal(status.as_u16(), message));
    }

    let data: Value = response.json().await.map_err(|e| {
        ProxyError::ForwardFailed(format!("Claude token refresh: unreadable reply: {e}"))
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

    propagate_refreshed_login(provider, &renewed, &refresh_token);
    log::info!("[claude_pool] login refreshed for provider={}", provider.id);
    Ok(renewed)
}

#[cfg(not(test))]
fn token_endpoint() -> String {
    TOKEN_ENDPOINT.to_string()
}

/// Tests renew against a local endpoint, or one that refuses connections, and
/// never against Anthropic's.
#[cfg(test)]
static TEST_TOKEN_ENDPOINT: Mutex<Option<String>> = Mutex::new(None);

#[cfg(test)]
fn token_endpoint() -> String {
    TEST_TOKEN_ENDPOINT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:9/v1/oauth/token".to_string())
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

/// A refresh spends the old refresh token everywhere it is held. The live
/// store gets the renewed login when the recorded owner says the live login is
/// this account's, or when it held the token just spent — only its
/// `claudeAiOauth` block, so the machine's own `mcpOAuth` keys stay — and the
/// marker then names this card with the new expiry. WSL gets it when it is
/// signed in to the account, even when that is not the account in use, or
/// when it held the spent token. An install left holding the spent token
/// would post it at its next renewal.
fn propagate_refreshed_login(provider: &Provider, renewed: &Value, spent: &str) {
    let Some(oauth) = renewed.get("claudeAiOauth") else {
        return;
    };
    let held_spent =
        |login: Option<&Value>| login.and_then(refresh_token).as_deref() == Some(spent);
    let wsl = mirror_credentials_of(provider).or_else(|| {
        wsl_credentials_path().filter(|path| held_spent(read_login_at(path).as_ref()))
    });
    if let Some(path) = wsl {
        match crate::services::credential_mirror::propagate(oauth, &path) {
            Ok(()) => log::info!("[claude_pool] wrote the refreshed login through to WSL"),
            Err(e) => log::warn!("[claude_pool] could not update the WSL login: {e}"),
        }
    }
    let live = read_live_login();
    if !live_is_owned_by(provider) && !held_spent(live.as_ref()) {
        return;
    }
    let mut live = live.filter(Value::is_object).unwrap_or_else(|| json!({}));
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

/// The device id this pooled account presents upstream: 64 hex characters,
/// made the first time the account serves a request and kept in the account's
/// folder, so every request it serves carries the same one and no two
/// accounts share one.
pub fn device_id_for(provider_id: &str) -> Option<String> {
    static GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
    let _guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let path = paths::snapshot_dir(provider_id).join("device_id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if existing.len() == 64 && existing.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(existing.to_string());
        }
    }
    let fresh = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    match store::write_snapshot_atomic(&path, fresh.as_bytes()) {
        Ok(()) => Some(fresh),
        Err(e) => {
            log::warn!("[claude_pool] could not store the device id for provider={provider_id}: {e}");
            None
        }
    }
}

/// The session id `provider_id` shows for the conversation Claude Code calls
/// `session`: SHA-256 of `provider_id::session`, formatted as a v4 UUID. The
/// same conversation gets a different id on every account, and the same one
/// each time it returns to an account.
pub fn derived_session_id(provider_id: &str, session: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(format!("{provider_id}::{session}").as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&hash[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b).to_string()
}

/// Claude Code writes the signed-in account's id, its device id and the
/// conversation's session id into `metadata.user_id`. When a pooled account's
/// login is presented, all three are replaced with that account's own: its
/// account id, its device id, and the session id derived for it, so the body
/// and the token agree and no account sees another's device or session. Both
/// shapes Claude Code has used are handled: the JSON string, and the older
/// `user_<device>_account_<uuid>_session_<uuid>` form. Returns the session id
/// now in the body, for the header that repeats it, or `None` when the body
/// carries no recognizable `user_id` (nothing changed).
pub fn patch_identity(
    body: &mut Value,
    provider_id: &str,
    account_uuid: &str,
    device_id: &str,
) -> Option<String> {
    let user_id = body
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
        .map(str::to_string)?;
    let (patched, session) = if let Ok(mut inner) = serde_json::from_str::<Value>(&user_id) {
        let obj = inner.as_object_mut()?;
        if !obj.contains_key("account_uuid") && !obj.contains_key("session_id") {
            return None;
        }
        let original = obj
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let session = derived_session_id(provider_id, &original);
        obj.insert("device_id".into(), json!(device_id));
        obj.insert("account_uuid".into(), json!(account_uuid));
        obj.insert("session_id".into(), json!(session));
        (serde_json::to_string(&inner).ok()?, session)
    } else {
        let account_at = user_id.find("_account_")?;
        let uuid_start = account_at + "_account_".len();
        let session_at = user_id[uuid_start..]
            .find("_session_")
            .map(|i| uuid_start + i)?;
        let original = &user_id[session_at + "_session_".len()..];
        let session = derived_session_id(provider_id, original);
        (
            format!("user_{device_id}_account_{account_uuid}_session_{session}"),
            session,
        )
    };
    body["metadata"]["user_id"] = json!(patched);
    Some(session)
}

// ── Quota ───────────────────────────────────────────────────────────────────

/// What one response's `anthropic-ratelimit-unified-*` headers say.
#[derive(Debug, Default)]
pub struct QuotaReport {
    pub windows: Vec<QuotaWindow>,
    /// The account as a whole was refused, until this time.
    pub limited_until: Option<i64>,
    /// Model-scoped windows that were refused, with their reset.
    pub scoped_limits: Vec<(String, i64)>,
}

/// Reads the `anthropic-ratelimit-unified-*` headers. Utilization is a 0–1
/// fraction; resets are unix seconds. Buckets are named by duration (`5h`,
/// `7d`); a bucket with a suffix (`7d_oi`) is model-scoped: a response
/// carries it only for the models it applies to, and its refusal does not
/// spend the account for other models.
pub fn parse_quota_headers(headers: &http::HeaderMap) -> QuotaReport {
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
    let mut scoped_limits = Vec::new();
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
        if p.rejected {
            let until = p.reset.filter(|r| *r > now).unwrap_or(now + 300);
            if account_wide {
                limited_until = Some(until);
            } else {
                scoped_limits.push((bucket.clone(), until));
            }
        }
        windows.push(QuotaWindow {
            limit_name: (!account_wide).then(|| bucket.clone()),
            window_minutes: minutes,
            used_percent: utilization * 100.0,
            reset_at: p.reset.filter(|r| *r > 0),
        });
    }
    // The overall status is `rejected` also when only a model-scoped bucket
    // refused; that one is already recorded against its model.
    if overall_rejected && limited_until.is_none() && scoped_limits.is_empty() {
        limited_until = Some(overall_reset.filter(|r| *r > now).unwrap_or(now + 300));
    }
    windows.sort_by(|a, b| {
        (a.limit_name.is_some(), &a.limit_name, a.window_minutes).cmp(&(
            b.limit_name.is_some(),
            &b.limit_name,
            b.window_minutes,
        ))
    });
    QuotaReport {
        windows,
        limited_until,
        scoped_limits,
    }
}

/// Records what a response for `model` said about the account that served
/// it: its windows, and any refusal, account-wide or for that model only.
pub fn record_quota(provider_id: &str, model: Option<&str>, headers: &http::HeaderMap) {
    let report = parse_quota_headers(headers);
    if !report.windows.is_empty() {
        account_pool::record_windows(provider_id, model, report.windows);
    }
    if let Some(until) = report.limited_until {
        account_pool::record_limited_until(provider_id, until);
    }
    for (window, until) in report.scoped_limits {
        account_pool::record_scoped_limited_until(provider_id, &window, until);
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
    fn only_a_rejected_token_makes_a_refresh_failure_a_login_problem() {
        for status in [400, 401, 403] {
            assert!(matches!(
                refresh_refusal(status, String::new()),
                ProxyError::AuthError(_)
            ));
        }
        for status in [429, 500, 502, 503, 504] {
            assert!(matches!(
                refresh_refusal(status, String::new()),
                ProxyError::ForwardFailed(_)
            ));
        }
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

    const DEV: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    #[test]
    fn identity_is_replaced_inside_the_json_user_id() {
        let mut body = json!({
            "model": "claude",
            "metadata": { "user_id": "{\"device_id\":\"d\",\"account_uuid\":\"aaaaaaaa-0000-0000-0000-000000000000\",\"session_id\":\"s\"}" }
        });
        let session = patch_identity(&mut body, "prov-a", "bbbbbbbb-0000-0000-0000-000000000000", DEV)
            .unwrap();
        let inner: Value =
            serde_json::from_str(body["metadata"]["user_id"].as_str().unwrap()).unwrap();
        assert_eq!(inner["account_uuid"], "bbbbbbbb-0000-0000-0000-000000000000");
        assert_eq!(inner["device_id"], DEV);
        assert_eq!(inner["session_id"], session.as_str());
        assert_eq!(session, derived_session_id("prov-a", "s"));
        assert_ne!(session, derived_session_id("prov-b", "s"));
        assert_ne!(session, "s");
    }

    #[test]
    fn identity_is_replaced_in_the_legacy_user_id() {
        let mut body = json!({ "metadata": { "user_id": "user_abc_account_old-uuid_session_s1" } });
        let session = patch_identity(&mut body, "prov-a", "new-uuid", DEV).unwrap();
        assert_eq!(
            body["metadata"]["user_id"],
            format!("user_{DEV}_account_new-uuid_session_{session}")
        );
        assert_eq!(session, derived_session_id("prov-a", "s1"));
        let mut none = json!({ "metadata": { "user_id": "plain" } });
        assert!(patch_identity(&mut none, "p", "x", DEV).is_none());
        let mut missing = json!({ "model": "claude" });
        assert!(patch_identity(&mut missing, "p", "x", DEV).is_none());
    }

    #[test]
    fn derived_session_id_is_a_stable_v4_uuid() {
        let id = derived_session_id("prov-a", "s");
        assert_eq!(id, derived_session_id("prov-a", "s"));
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
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
        let report = parse_quota_headers(&headers(&[
            ("anthropic-ratelimit-unified-5h-utilization", "0.42"),
            ("anthropic-ratelimit-unified-5h-reset", "1900000000"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.9"),
            ("anthropic-ratelimit-unified-7d_oi-utilization", "1.1"),
            ("anthropic-ratelimit-unified-7d_oi-status", "rejected"),
            ("anthropic-ratelimit-unified-status", "rejected"),
        ]));
        assert_eq!(
            report.limited_until, None,
            "a model-scoped refusal does not spend the account"
        );
        assert_eq!(report.scoped_limits.len(), 1);
        assert_eq!(report.scoped_limits[0].0, "7d_oi");
        let windows = report.windows;
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
        let report = parse_quota_headers(&headers(&[
            ("anthropic-ratelimit-unified-5h-utilization", "1"),
            ("anthropic-ratelimit-unified-5h-status", "rejected"),
            ("anthropic-ratelimit-unified-5h-reset", "4102444800"),
            ("anthropic-ratelimit-unified-status", "rejected"),
        ]));
        assert_eq!(report.limited_until, Some(4_102_444_800));
        assert!(report.scoped_limits.is_empty());
    }

    // ── WSL holding the same account ────────────────────────────────────────

    struct WslHome {
        dir: tempfile::TempDir,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl WslHome {
        /// A sandboxed home with a WSL `.claude` folder configured as the mirror.
        fn new() -> Self {
            let dir = tempfile::TempDir::new().expect("temp home");
            let saved = ["HOME", "USERPROFILE", "SWITCHY_TEST_HOME"]
                .into_iter()
                .map(|key| {
                    let old = std::env::var(key).ok();
                    std::env::set_var(key, dir.path());
                    (key, old)
                })
                .collect();
            crate::settings::reload_settings().expect("reload settings");
            let home = Self { dir, saved };
            std::fs::create_dir_all(home.mirror_dir()).expect("mirror dir");
            crate::settings::set_claude_mirror_config_dir(Some(home.mirror_dir()))
                .expect("set mirror dir");
            home
        }

        fn mirror_dir(&self) -> std::path::PathBuf {
            self.dir.path().join("wsl").join(".claude")
        }

        fn sign_wsl_in(&self, uuid: &str, login: &Value) {
            let dir = self.mirror_dir();
            std::fs::write(
                paths::mirror_claude_config_path(&dir),
                json!({ "oauthAccount": { "accountUuid": uuid } }).to_string(),
            )
            .expect("wsl identity");
            std::fs::write(dir.join(".credentials.json"), login.to_string()).expect("wsl login");
        }

        fn wsl_login(&self) -> Value {
            let bytes = std::fs::read(self.mirror_dir().join(".credentials.json")).expect("read");
            serde_json::from_slice(&bytes).expect("parse")
        }
    }

    impl Drop for WslHome {
        fn drop(&mut self) {
            for (key, old) in &self.saved {
                match old {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            let _ = crate::settings::reload_settings();
        }
    }

    fn store_login(provider: &Provider, login: &Value) {
        let path = paths::snapshot_credentials_path(&provider.id);
        std::fs::create_dir_all(path.parent().unwrap()).expect("snapshot dir");
        std::fs::write(path, login.to_string()).expect("store login");
    }

    fn refuse(provider: &Provider, refresh: &str) {
        REFRESH_STATE
            .lock()
            .unwrap()
            .entry(provider.id.clone())
            .or_default()
            .dead_refresh_token = Some(refresh.to_string());
    }

    fn official_with_id(id: &str, uuid: &str) -> Provider {
        let mut p = official(Some(uuid));
        p.id = id.to_string();
        p
    }

    #[test]
    #[serial_test::serial]
    fn a_refused_login_is_replaced_by_a_newer_one_wsl_renewed() {
        let home = WslHome::new();
        let provider = official_with_id("wsl-heal", "u-1");
        store_login(&provider, &root("a-old", "r-old", 1_000));
        refuse(&provider, "r-old");
        let mut wsl = root("a-new", "r-new", 2_000);
        wsl["mcpOAuth"] = json!({ "wsl": true });
        home.sign_wsl_in("u-1", &wsl);

        assert!(!needs_sign_in(&provider));
        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-new"));
        assert_eq!(stored["mcpOAuth"], json!({ "keep": true }));
    }

    #[test]
    #[serial_test::serial]
    fn wsl_signed_in_to_another_account_is_not_taken() {
        let home = WslHome::new();
        let provider = official_with_id("wsl-other", "u-1");
        store_login(&provider, &root("a-old", "r-old", 1_000));
        refuse(&provider, "r-old");
        home.sign_wsl_in("u-2", &root("a-new", "r-new", 2_000));

        assert!(needs_sign_in(&provider));
        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-old"));
    }

    #[test]
    #[serial_test::serial]
    fn a_renewed_login_reaches_wsl_signed_in_to_that_account() {
        let home = WslHome::new();
        let provider = official_with_id("wsl-write", "u-1");
        let mut wsl = root("a-old", "r-old", 1_000);
        wsl["mcpOAuth"] = json!({ "wsl": true });
        home.sign_wsl_in("u-1", &wsl);

        propagate_refreshed_login(&provider, &root("a-new", "r-new", 2_000), "r-old");

        let written = home.wsl_login();
        assert_eq!(refresh_token(&written).as_deref(), Some("r-new"));
        assert_eq!(written["mcpOAuth"], json!({ "wsl": true }));
    }

    #[test]
    #[serial_test::serial]
    fn a_renewed_login_leaves_wsl_on_another_account_alone() {
        let home = WslHome::new();
        let provider = official_with_id("wsl-keep", "u-1");
        home.sign_wsl_in("u-2", &root("a-other", "r-other", 1_000));

        propagate_refreshed_login(&provider, &root("a-new", "r-new", 2_000), "r-old");

        assert_eq!(refresh_token(&home.wsl_login()).as_deref(), Some("r-other"));
    }

    // ── Leaving the renewal to Claude Code ──────────────────────────────────

    #[test]
    fn a_login_claude_code_holds_is_renewed_here_only_once_it_has_run_out() {
        let one = Holders::OneInstall;
        assert_eq!(renewal(9 * 60 * 1000, one, false), Renewal::Present);
        assert_eq!(renewal(4 * 60 * 1000, one, false), Renewal::Present);
        assert_eq!(renewal(1, one, false), Renewal::Present);
        assert_eq!(renewal(0, one, false), Renewal::Present);
        assert_eq!(renewal(-1, one, false), Renewal::AwaitHolder);
        assert_eq!(renewal(8 * 3_600_000, one, true), Renewal::AwaitHolder);
    }

    #[test]
    fn a_login_only_the_proxy_holds_is_renewed_five_minutes_ahead() {
        let proxy = Holders::ProxyOnly;
        assert_eq!(renewal(4 * 60 * 1000, proxy, false), Renewal::Renew);
        assert_eq!(renewal(6 * 60 * 1000, proxy, false), Renewal::Present);
        assert_eq!(renewal(9 * 60 * 1000, proxy, false), Renewal::Present);
        assert_eq!(renewal(8 * 3_600_000, proxy, true), Renewal::Renew);
    }

    #[test]
    fn a_login_both_installs_hold_is_renewed_here_before_their_window_and_never_in_it() {
        let both = Holders::BothInstalls;
        assert_eq!(renewal(11 * 60 * 1000, both, false), Renewal::Present);
        assert_eq!(renewal(10 * 60 * 1000, both, false), Renewal::Present);
        assert_eq!(renewal(10 * 60 * 1000 - 1, both, false), Renewal::Renew);
        assert_eq!(renewal(5 * 60 * 1000, both, false), Renewal::Renew);
        // The installs' own window: a renewal here would be a third poster.
        assert_eq!(renewal(5 * 60 * 1000 - 1, both, false), Renewal::Present);
        assert_eq!(renewal(0, both, false), Renewal::Present);
        assert_eq!(renewal(-1, both, false), Renewal::AwaitHolder);
        assert_eq!(renewal(8 * 3_600_000, both, true), Renewal::AwaitHolder);
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// Windows' Claude Code signed in to `provider`'s account: the live store
    /// holds `login` and the owner marker names the provider.
    fn sign_live_in(provider: &Provider, login: &Value) {
        let path = paths::live_credentials_path();
        std::fs::create_dir_all(path.parent().unwrap()).expect("live dir");
        std::fs::write(&path, login.to_string()).expect("live login");
        let (_, expires_at) = credential_health(login);
        claude_account::write_live_owner(&provider.id, account_uuid(provider).unwrap(), expires_at);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn the_proxy_does_not_renew_a_held_login_claude_code_is_about_to_renew() {
        let _home = WslHome::new();
        let provider = official_with_id("held-window", "u-1");
        let login = root("a-held", "r-held", now_ms() + 2 * 60 * 1000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);

        let token = access_token_for(&provider, false).await.expect("token");

        assert_eq!(token, "a-held");
        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-held"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_401_answered_by_a_renewal_claude_code_made_is_not_renewed_again() {
        let _home = WslHome::new();
        let provider = official_with_id("held-401-adopt", "u-1");
        store_login(&provider, &root("a-old", "r-old", now_ms() + 3_600_000));
        sign_live_in(&provider, &root("a-new", "r-new", now_ms() + 8 * 3_600_000));

        let token = access_token_for(&provider, true).await.expect("token");

        assert_eq!(token, "a-new");
        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-new"));
        // A second 401 from a request sent before the renewal is answered too.
        let again = access_token_for(&provider, true).await.expect("token");
        assert_eq!(again, "a-new");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_401_on_a_held_login_waits_for_claude_codes_renewal_and_takes_it() {
        let _home = WslHome::new();
        let provider = official_with_id("held-401-wait", "u-1");
        let login = root("a-old", "r-old", now_ms() + 3_600_000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);

        let renewer = provider.clone();
        let renewal_lands = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            sign_live_in(&renewer, &root("a-new", "r-new", now_ms() + 8 * 3_600_000));
        });
        let token = access_token_for(&provider, true).await.expect("token");
        renewal_lands.await.expect("renewal task");

        assert_eq!(token, "a-new");
        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-new"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_refused_held_login_fails_at_once_without_waiting() {
        let _home = WslHome::new();
        let provider = official_with_id("held-refused", "u-1");
        let login = root("a-dead", "r-dead", now_ms() - 1_000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);
        refuse(&provider, "r-dead");

        let started = std::time::Instant::now();
        let result = access_token_for(&provider, false).await;

        assert!(matches!(result, Err(ProxyError::AuthError(_))));
        assert!(started.elapsed() < std::time::Duration::from_millis(HELD_RENEWAL_POLL_MS));
    }

    #[test]
    fn a_refused_refresh_token_is_known_whichever_provider_held_it() {
        let provider = official_with_id("refused-any", "u-1");
        refuse(&provider, "r-spent-anywhere");
        assert!(refresh_token_was_refused("r-spent-anywhere"));
        assert!(!refresh_token_was_refused("r-never-refused"));
        assert!(!refresh_token_was_refused(""));
    }

    // ── One renewer for a login both installs hold ──────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A token endpoint on 127.0.0.1 that answers each renewal with a new
    /// login and counts the renewals it was sent.
    struct TokenEndpoint {
        renewals: Arc<AtomicUsize>,
    }

    impl TokenEndpoint {
        async fn start() -> Self {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let url = format!(
                "http://{}/v1/oauth/token",
                listener.local_addr().expect("addr")
            );
            let renewals = Arc::new(AtomicUsize::new(0));
            let counter = renewals.clone();
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let counter = counter.clone();
                    tokio::spawn(async move {
                        let mut request = Vec::new();
                        let mut buf = [0u8; 4096];
                        while !request_complete(&request) {
                            match socket.read(&mut buf).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => request.extend_from_slice(&buf[..n]),
                            }
                        }
                        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        let body = json!({
                            "access_token": format!("a-renewed-{n}"),
                            "refresh_token": format!("r-renewed-{n}"),
                            "expires_in": 28_800,
                        })
                        .to_string();
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(reply.as_bytes()).await;
                    });
                }
            });
            *TEST_TOKEN_ENDPOINT.lock().unwrap() = Some(url);
            Self { renewals }
        }

        fn renewals(&self) -> usize {
            self.renewals.load(Ordering::SeqCst)
        }
    }

    impl Drop for TokenEndpoint {
        fn drop(&mut self) {
            *TEST_TOKEN_ENDPOINT.lock().unwrap() = None;
        }
    }

    fn request_complete(request: &[u8]) -> bool {
        let Some(head_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
            return false;
        };
        let head = String::from_utf8_lossy(&request[..head_end]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        request.len() >= head_end + 4 + length
    }

    fn live_login() -> Value {
        read_live_login().expect("live login")
    }

    #[test]
    #[serial_test::serial]
    fn an_install_carrying_the_same_refresh_token_holds_the_login_without_a_marker() {
        let home = WslHome::new();
        let provider = official_with_id("holders", "u-1");
        let login = root("a-1", "r-1", 1_000);
        assert_eq!(holders(&provider, &login), Holders::ProxyOnly);

        // A capture leaves the live store holding the card's login with the
        // marker still naming whichever card was switched to last.
        let path = paths::live_credentials_path();
        std::fs::create_dir_all(path.parent().unwrap()).expect("live dir");
        std::fs::write(&path, login.to_string()).expect("live login");
        assert_eq!(holders(&provider, &login), Holders::OneInstall);

        // WSL signed in to another account but carrying the same login (a
        // copy between installs) holds it too.
        home.sign_wsl_in("u-2", &login);
        assert_eq!(holders(&provider, &login), Holders::BothInstalls);

        // A different login of the same account is not this one.
        assert_eq!(
            holders(&provider, &root("a-2", "r-2", 2_000)),
            Holders::ProxyOnly
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_login_both_installs_hold_is_renewed_once_ahead_of_them_and_written_to_both() {
        let home = WslHome::new();
        let endpoint = TokenEndpoint::start().await;
        let provider = official_with_id("both-ahead", "u-1");
        let login = root("a-shared", "r-shared", now_ms() + 8 * 60 * 1000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);
        home.sign_wsl_in("u-1", &login);

        let token = access_token_for(&provider, false).await.expect("token");

        assert_eq!(token, "a-renewed-1");
        assert_eq!(endpoint.renewals(), 1);
        for (holder, held) in [
            ("stored", read_stored(&provider.id).expect("stored")),
            ("live", live_login()),
            ("wsl", home.wsl_login()),
        ] {
            assert_eq!(
                refresh_token(&held).as_deref(),
                Some("r-renewed-1"),
                "{holder}"
            );
        }
        // The installs now hold a login hours from their window.
        let again = access_token_for(&provider, false).await.expect("token");
        assert_eq!(again, "a-renewed-1");
        assert_eq!(endpoint.renewals(), 1);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn the_window_of_both_installs_is_left_to_them() {
        let home = WslHome::new();
        let endpoint = TokenEndpoint::start().await;
        let provider = official_with_id("both-window", "u-1");
        let login = root("a-shared", "r-shared", now_ms() + 3 * 60 * 1000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);
        home.sign_wsl_in("u-1", &login);

        let token = access_token_for(&provider, false).await.expect("token");

        assert_eq!(token, "a-shared");
        assert_eq!(endpoint.renewals(), 0);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_login_one_install_holds_is_not_renewed_ahead_of_it() {
        let _home = WslHome::new();
        let endpoint = TokenEndpoint::start().await;
        let provider = official_with_id("one-ahead", "u-1");
        let login = root("a-held", "r-held", now_ms() + 8 * 60 * 1000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);

        let token = access_token_for(&provider, false).await.expect("token");

        assert_eq!(token, "a-held");
        assert_eq!(endpoint.renewals(), 0);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn claude_codes_renewal_of_a_login_the_marker_does_not_name_is_taken_not_raced() {
        let _home = WslHome::new();
        let endpoint = TokenEndpoint::start().await;
        let provider = official_with_id("carried", "u-1");
        let other = official_with_id("marker-other", "u-9");
        let login = root("a-old", "r-old", now_ms() + 2 * 60 * 1000);
        store_login(&provider, &login);
        // The live store holds the card's login; the marker names another card.
        sign_live_in(&other, &login);

        // Claude Code's window: the proxy presents the login and records the
        // owner instead of renewing alongside Claude Code.
        let token = access_token_for(&provider, false).await.expect("token");
        assert_eq!(token, "a-old");
        let owner = claude_account::read_live_owner().expect("owner");
        assert_eq!(owner.provider_id, provider.id);

        // Claude Code renews; a 401 on the old token is answered by that
        // renewal, and the spent token is never posted.
        std::fs::write(
            paths::live_credentials_path(),
            root("a-new", "r-new", now_ms() + 8 * 3_600_000).to_string(),
        )
        .expect("renewal");
        let token = access_token_for(&provider, true).await.expect("token");
        assert_eq!(token, "a-new");
        assert_eq!(endpoint.renewals(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn a_renewal_reaches_every_install_that_held_the_spent_token() {
        let home = WslHome::new();
        let provider = official_with_id("spent-everywhere", "u-1");
        let other = official_with_id("marker-elsewhere", "u-9");
        let spent = root("a-old", "r-old", 1_000);
        // The live store holds the card's login while the marker names
        // another card; WSL is signed in to another account with a copy.
        sign_live_in(&other, &spent);
        home.sign_wsl_in("u-2", &spent);

        propagate_refreshed_login(&provider, &root("a-new", "r-new", 2_000), "r-old");

        assert_eq!(refresh_token(&live_login()).as_deref(), Some("r-new"));
        assert_eq!(refresh_token(&home.wsl_login()).as_deref(), Some("r-new"));
        let owner = claude_account::read_live_owner().expect("owner");
        assert_eq!(owner.provider_id, provider.id);
    }

    #[test]
    #[serial_test::serial]
    fn a_renewal_leaves_installs_holding_other_logins_alone() {
        let home = WslHome::new();
        let provider = official_with_id("spent-nowhere", "u-1");
        let other = official_with_id("live-other", "u-9");
        sign_live_in(&other, &root("a-x", "r-x", 1_000));
        home.sign_wsl_in("u-2", &root("a-y", "r-y", 1_000));

        propagate_refreshed_login(&provider, &root("a-new", "r-new", 2_000), "r-old");

        assert_eq!(refresh_token(&live_login()).as_deref(), Some("r-x"));
        assert_eq!(refresh_token(&home.wsl_login()).as_deref(), Some("r-y"));
        let owner = claude_account::read_live_owner().expect("owner");
        assert_eq!(owner.provider_id, other.id);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn the_reconciler_renews_a_shared_login_through_the_card_that_holds_it() {
        let home = WslHome::new();
        let endpoint = TokenEndpoint::start().await;
        let db = crate::database::Database::memory().expect("db");
        let provider = official_with_id("shared-card", "u-1");
        db.save_provider("claude", &provider).expect("save");
        let login = root("a-shared", "r-shared", now_ms() + 8 * 60 * 1000);
        store_login(&provider, &login);
        sign_live_in(&provider, &login);
        home.sign_wsl_in("u-1", &login);

        renew_shared_login(&db, "r-shared").await;

        assert_eq!(endpoint.renewals(), 1);
        assert_eq!(refresh_token(&live_login()).as_deref(), Some("r-renewed-1"));
        assert_eq!(
            refresh_token(&home.wsl_login()).as_deref(),
            Some("r-renewed-1")
        );

        // A login no card holds is left to the installs.
        renew_shared_login(&db, "r-uncaptured").await;
        assert_eq!(endpoint.renewals(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn a_renewal_wsl_made_is_kept_before_a_switch_overwrites_it() {
        let home = WslHome::new();
        let provider = official_with_id("wsl-switch", "u-1");
        store_login(&provider, &root("a-old", "r-old", 1_000));
        home.sign_wsl_in("u-1", &root("a-new", "r-new", 2_000));

        take_newer_wsl_login(&provider);

        let stored = read_stored(&provider.id).expect("stored");
        assert_eq!(refresh_token(&stored).as_deref(), Some("r-new"));
    }
}
