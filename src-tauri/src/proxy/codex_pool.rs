//! ChatGPT-login Codex accounts behind the proxy.
//!
//! A running Codex session loads `~/.codex/auth.json` once and refuses to
//! reload a login that belongs to a different account, so swapping the file
//! cannot change the account of a session that is already open. Routing Codex
//! through the local proxy can: the proxy drops the login Codex sends and
//! presents the login of whichever Official provider is selected.
//!
//! This module holds the three things that takes:
//!
//!   * recognising a provider that carries a ChatGPT login rather than an API
//!     key, and producing the credentials to present for it,
//!   * keeping that login usable — refreshing it when its access token runs
//!     out, under the same newest-valid-login-wins rules the switch-away
//!     backfill and the WSL reconciler apply, so the proxy never becomes a
//!     second holder that rotates the refresh token away from Codex itself,
//!   * reading the quota OpenAI reports on every response, which
//!     `account_pool` uses to move the failover queue off a spent account.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::account_pool::{self, QuotaWindow};
use crate::proxy::error::ProxyError;
use crate::services::codex_account::{self, BackfillVerdict};

/// Where a ChatGPT login is served from. Not `api.openai.com`: the token is a
/// ChatGPT credential, and Codex appends `/responses` and `/models` to this.
pub const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Path prefix Codex uses against the proxy when its built-in provider is
/// pointed here with `openai_base_url`.
pub const BACKEND_PATH_PREFIX: &str = "/backend-api/codex";

/// Asked, before a ChatGPT login is used, where this machine is seen from.
pub const EXIT_TRACE_URL: &str = "https://chatgpt.com/cdn-cgi/trace";

const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
/// The Codex CLI's own OAuth client: the one these logins were issued to.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Refresh when the access token has less than this left.
const REFRESH_WINDOW_SECS: i64 = 300;
/// A 401 that arrives this soon after a refresh is about a request sent before
/// it; refreshing again would only rotate the token a second time.
const FORCED_REFRESH_FLOOR_SECS: i64 = 60;

// ── Recognising a ChatGPT-login provider ────────────────────────────────────

/// True when the provider authenticates with a stored ChatGPT login: it has
/// one, and neither an API key nor a third-party endpoint of its own.
pub fn is_chatgpt_provider(provider: &Provider) -> bool {
    let settings = &provider.settings_config;
    let Some(auth) = settings.get("auth") else {
        return false;
    };
    if codex_account::inspect(auth).is_none() {
        return false;
    }
    let has_api_key = auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|k| !k.is_empty() && k != "PROXY_MANAGED");
    if has_api_key {
        return false;
    }
    let has_own_endpoint = settings
        .get("config")
        .and_then(Value::as_str)
        .is_some_and(|toml| toml.contains("base_url"));
    !has_own_endpoint
}

/// True when a live `auth.json` value is a ChatGPT login with no API key, i.e.
/// Codex itself is running in ChatGPT mode.
pub fn is_chatgpt_live_auth(auth: &Value) -> bool {
    if codex_account::inspect(auth).is_none() {
        return false;
    }
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
}

/// The credentials to present upstream for one account.
#[derive(Debug, Clone)]
pub struct ChatgptCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

fn credentials_from(auth: &Value) -> Option<ChatgptCredentials> {
    let tokens = auth.get("tokens")?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();
    let account_id = codex_account::inspect(auth).and_then(|l| l.account_id);
    Some(ChatgptCredentials {
        access_token,
        account_id,
    })
}

/// Unix seconds at which the access token expires; `None` when unreadable.
fn access_token_expiry(auth: &Value) -> Option<i64> {
    auth.get("tokens")?
        .get("access_token")
        .and_then(Value::as_str)
        .and_then(codex_account::decode_jwt_payload)?
        .get("exp")?
        .as_i64()
}

// ── Keeping a login usable ──────────────────────────────────────────────────

#[derive(Default)]
struct RefreshState {
    /// Refresh token OpenAI has rejected. Re-sending it can only fail again.
    dead_refresh_token: Option<String>,
    last_refresh_at: i64,
}

static REFRESH_STATE: Lazy<Mutex<HashMap<String, RefreshState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Whether OpenAI has refused `provider`'s stored refresh token, so the
/// account stays unusable until someone signs it in again. A new login
/// carries a new refresh token and clears this.
pub fn needs_sign_in(provider: &Provider) -> bool {
    let Some(refresh_token) = stored_auth(provider)
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    state
        .get(&provider.id)
        .and_then(|s| s.dead_refresh_token.as_deref())
        == Some(refresh_token.as_str())
}

/// One refresh at a time per provider; concurrent requests wait for it.
static REFRESH_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn refresh_lock(provider_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = REFRESH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(provider_id.to_string()).or_default().clone()
}

/// Puts `provider`'s login into Codex's saved login (`auth.json`, and the WSL
/// mirror's) after that account has answered a request, so new sessions and
/// `/status` show the account the proxy serves. Only a login that just worked
/// is written: Codex reads the saved login at startup, straight from OpenAI,
/// and exits on a refused one. A login Codex renewed on its own is filed with
/// its account first.
pub fn save_login_of_serving_account(db: &Database, provider: &Provider) {
    let live = read_live_auth();
    let Some(login) = db
        .get_provider_by_id(&provider.id, "codex")
        .ok()
        .flatten()
        .map(|p| stored_auth(&p))
        .filter(|auth| codex_account::inspect(auth).is_some())
    else {
        return;
    };
    let key = |auth: &Value| {
        codex_account::inspect(auth).and_then(|l| l.account_key().map(str::to_string))
    };
    if key(&live).is_none() || key(&live) != key(&login) {
        file_live_login(db);
        let login = db
            .get_provider_by_id(&provider.id, "codex")
            .ok()
            .flatten()
            .map(|p| stored_auth(&p))
            .unwrap_or(login);
        let mut paths = vec![crate::codex_config::get_codex_auth_path()];
        if let Some(dir) = crate::settings::get_codex_mirror_override_dir() {
            paths.push(dir.join("auth.json"));
        }
        // A signed-out Codex has no auth.json; the login goes back wherever
        // the install's folder is.
        for path in paths
            .into_iter()
            .filter(|p| p.parent().is_some_and(std::path::Path::exists))
        {
            let current: Value = crate::config::read_json_file(&path).unwrap_or(Value::Null);
            let next = codex_account::transplant_login(&current, &login);
            if let Err(e) = crate::config::write_json_file(&path, &next) {
                log::warn!(
                    "[codex_pool] could not save the login to {}: {e}",
                    path.display()
                );
                return;
            }
        }
        log::info!(
            "[codex_pool] Codex's saved login is now provider={} (it answered through the proxy)",
            provider.id
        );
    }
}

/// Signs Codex out when `provider` is the account picked by hand (held) and
/// OpenAI has refused its login, the way `codex logout` does: Codex's
/// `auth.json` (Windows and the WSL mirror) is deleted, so the next Codex
/// started opens on its own sign-in screen. Codex treats any `auth.json`,
/// even an empty one, as a ChatGPT login and exits at startup when it is
/// unusable. It runs at the pick when the refusal is already known, and when
/// a refusal is learned while the pick holds; a running Codex session has no
/// sign-in of its own. A file holding an API key, or a new sign-in of this
/// account not yet filed with it, is kept. The logins stay stored with their
/// providers; a working account's login is put back once it answers
/// (`save_login_of_serving_account`).
pub fn sign_codex_out_for(provider: &Provider) {
    if super::manual_hold::held("codex").as_deref() != Some(provider.id.as_str())
        || !needs_sign_in(provider)
    {
        return;
    }
    let refused = stored_auth(provider);
    // WSL's first: the mirror's reconcile, woken by the Windows file going,
    // would otherwise copy WSL's login back.
    let mut paths: Vec<_> = crate::settings::get_codex_mirror_override_dir()
        .map(|dir| dir.join("auth.json"))
        .into_iter()
        .collect();
    paths.push(crate::codex_config::get_codex_auth_path());
    let mut signed_out = false;
    for path in paths.into_iter().filter(|p| p.exists()) {
        let auth: Value = crate::config::read_json_file(&path).unwrap_or(Value::Null);
        if holds_api_key(&auth) || is_new_sign_in_of(&auth, &refused) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => signed_out = true,
            Err(e) => log::warn!(
                "[codex_pool] could not sign Codex out at {}: {e}",
                path.display()
            ),
        }
    }
    if signed_out {
        log::info!(
            "[codex_pool] signed Codex out: provider={} was picked by hand and its login was refused",
            provider.id
        );
    }
}

fn holds_api_key(auth: &Value) -> bool {
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.trim().is_empty())
}

/// True when `auth` is a login of the same account as `refused` with another
/// refresh token: the account was signed in again.
fn is_new_sign_in_of(auth: &Value, refused: &Value) -> bool {
    let refresh = |a: &Value| {
        a.get("tokens")
            .and_then(|t| t.get("refresh_token"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let key =
        |a: &Value| codex_account::inspect(a).and_then(|l| l.account_key().map(str::to_string));
    key(auth).is_some() && key(auth) == key(refused) && refresh(auth) != refresh(refused)
}

static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Lets a sign-in filed with a card reach the window and the proxy.
pub fn set_app_handle(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// A new sign-in filed with a card clears what the refused login left behind,
/// its failure count and circuit breaker, and tells the window, so the card
/// shows the account signed in without waiting for a request.
fn announce_sign_in(provider_id: &str) {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let id = provider_id.to_string();
    tauri::async_runtime::spawn(async move {
        use tauri::{Emitter, Manager};
        let state = app.state::<crate::store::AppState>();
        if let Err(e) = state
            .db
            .update_provider_health(&id, "codex", true, None)
            .await
        {
            log::warn!("[codex_pool] could not clear the failures of provider={id}: {e}");
        }
        let _ = state
            .proxy_service
            .reset_provider_circuit_breaker(&id, "codex")
            .await;
        super::codex_engine::nudge();
        let payload = json!({ "appType": "codex", "providerId": id });
        if let Err(e) = app.emit("account-signed-in", payload) {
            log::warn!("[codex_pool] could not announce the sign-in: {e}");
        }
    });
}

fn read_live_auth() -> Value {
    let path = crate::codex_config::get_codex_auth_path();
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

fn stored_auth(provider: &Provider) -> Value {
    provider
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or(Value::Null)
}

/// If Codex's own `auth.json` holds a newer login for the same account, take
/// it: Codex refreshed, and the stored copy's refresh token is now spent.
fn adopt_newer_live_login(db: &Database, provider: &mut Provider) {
    let stored = stored_auth(provider);
    let live = read_live_auth();
    let (Some(stored_login), Some(live_login)) = (
        codex_account::inspect(&stored),
        codex_account::inspect(&live),
    ) else {
        return;
    };
    if !live_login.alive
        || stored_login.account_key().is_none()
        || stored_login.account_key() != live_login.account_key()
        || live_login.last_refresh <= stored_login.last_refresh
    {
        return;
    }
    if codex_account::judge_backfill(&stored, &live) != BackfillVerdict::Accept {
        return;
    }
    let was_refused = needs_sign_in(provider);
    let merged = codex_account::transplant_login(&stored, &live);
    if let Some(obj) = provider.settings_config.as_object_mut() {
        obj.insert("auth".to_string(), merged);
    }
    match db.save_provider("codex", provider) {
        Ok(()) => {
            log::info!(
                "[codex_pool] provider={} took the newer login from auth.json",
                provider.id
            );
            if was_refused {
                announce_sign_in(&provider.id);
            }
        }
        Err(e) => log::warn!(
            "[codex_pool] could not store the newer live login for provider={}: {e}",
            provider.id
        ),
    }
}

/// A `codex login` run while the proxy serves Codex only reaches `auth.json`:
/// switching is a hot switch then, so no switch-away backfill ever reads it.
/// Called before each Codex request is routed, this files it under the rules
/// that backfill applies. The provider holding that account takes a newer
/// login of it; a login of an account no provider holds goes to the current
/// Official provider when that holds no usable login of its own — the one
/// enabled so it could be signed in. While Codex on Windows has no login, a
/// sign-in made in WSL is carried over first, so the request it sent is
/// served on it.
pub fn file_live_login(db: &Database) {
    let mut live = read_live_auth();
    if !codex_account::inspect(&live).is_some_and(|l| l.alive) {
        crate::services::credential_mirror::reconcile_codex();
        live = read_live_auth();
    }
    let Ok(providers) = db.get_all_providers("codex") else {
        return;
    };
    let current_id =
        crate::settings::get_effective_current_provider(db, &crate::app_config::AppType::Codex)
            .ok()
            .flatten();
    let providers: Vec<Provider> = providers.into_values().collect();

    match live_login_home(&providers, current_id.as_deref(), &live) {
        None => {}
        Some(LiveLoginHome::Holder(id)) => {
            if let Some(mut holder) = providers.into_iter().find(|p| p.id == id) {
                adopt_newer_live_login(db, &mut holder);
            }
        }
        Some(LiveLoginHome::Current(id)) => {
            let Some(mut current) = providers.into_iter().find(|p| p.id == id) else {
                return;
            };
            let merged = codex_account::transplant_login(&stored_auth(&current), &live);
            if let Some(obj) = current.settings_config.as_object_mut() {
                obj.insert("auth".to_string(), merged);
            }
            match db.save_provider("codex", &current) {
                Ok(()) => {
                    log::info!(
                        "[codex_pool] provider={} took the new login from auth.json",
                        current.id
                    );
                    announce_sign_in(&current.id);
                }
                Err(e) => log::warn!(
                    "[codex_pool] could not store the new login under provider={}: {e}",
                    current.id
                ),
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LiveLoginHome {
    /// This provider already holds the account; its own rules decide.
    Holder(String),
    /// No provider holds the account; the current one, holding no usable
    /// login, takes it.
    Current(String),
}

/// Which provider a live login belongs to, if any.
fn live_login_home(
    providers: &[Provider],
    current_id: Option<&str>,
    live: &Value,
) -> Option<LiveLoginHome> {
    let live_login = codex_account::inspect(live).filter(|l| l.alive)?;
    let live_key = live_login.account_key()?;

    let key_of = |p: &Provider| {
        codex_account::inspect(&stored_auth(p)).and_then(|l| l.account_key().map(str::to_string))
    };
    if let Some(holder) = providers
        .iter()
        .find(|p| key_of(p).as_deref() == Some(live_key))
    {
        return Some(LiveLoginHome::Holder(holder.id.clone()));
    }

    let current = providers
        .iter()
        .find(|p| Some(p.id.as_str()) == current_id)?;
    let stored = stored_auth(current);
    let has_api_key = stored
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|k| !k.is_empty() && k != "PROXY_MANAGED");
    let holds_usable_login = codex_account::inspect(&stored).is_some_and(|l| l.alive);
    (current.category.as_deref() == Some("official") && !has_api_key && !holds_usable_login)
        .then(|| LiveLoginHome::Current(current.id.clone()))
}

/// When the proxy hands Codex's config back, Codex keeps the login it has
/// if that is a ChatGPT login: under the proxy it is the login of the last
/// account that answered (`save_login_of_serving_account`), or a login Codex
/// renewed or signed in itself. `incoming` (the current provider's settings)
/// supplies the login only when Codex has none, so a refused login the
/// current provider holds is not handed to Codex, which could not start on
/// it. Logins Codex renewed are filed with their accounts first.
pub fn keep_live_login(db: &Database, incoming: &mut Value) {
    if let Ok(providers) = db.get_all_providers("codex") {
        for mut provider in providers.into_values() {
            adopt_newer_live_login(db, &mut provider);
        }
    }

    let live = read_live_auth();
    if !codex_account::inspect(&live).is_some_and(|login| login.alive) {
        return;
    }
    let incoming_auth = incoming.get("auth").cloned().unwrap_or(Value::Null);
    if let Some(obj) = incoming.as_object_mut() {
        obj.insert(
            "auth".to_string(),
            codex_account::transplant_login(&incoming_auth, &live),
        );
    }
}

/// Credentials for `provider`, refreshed first when the access token is about
/// to run out. `force` refreshes regardless, in answer to a 401.
pub async fn credentials_for(
    db: &Arc<Database>,
    provider: &Provider,
    force: bool,
) -> Result<ChatgptCredentials, ProxyError> {
    let lock = refresh_lock(&provider.id);
    let _guard = lock.lock().await;

    // Re-read under the lock: a request that waited here should see what the
    // refresh before it stored.
    let mut current = db
        .get_provider_by_id(&provider.id, "codex")
        .ok()
        .flatten()
        .unwrap_or_else(|| provider.clone());
    adopt_newer_live_login(db, &mut current);

    let auth = stored_auth(&current);
    let now = chrono::Utc::now().timestamp();
    let expiring = access_token_expiry(&auth).is_none_or(|exp| exp - now < REFRESH_WINDOW_SECS);

    let recently_refreshed = {
        let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .get(&current.id)
            .is_some_and(|s| now - s.last_refresh_at < FORCED_REFRESH_FLOOR_SECS)
    };

    if expiring || (force && !recently_refreshed) {
        match refresh_login(db, &mut current).await {
            Ok(()) => {}
            Err(e) if expiring => return Err(e),
            // A forced refresh that fails leaves a token that may still work
            // for other requests; the caller relays the original 401.
            Err(e) => log::warn!("[codex_pool] forced refresh failed: {e}"),
        }
    }

    credentials_from(&stored_auth(&current)).ok_or_else(|| {
        ProxyError::AuthError(format!(
            "Codex provider {} has no ChatGPT login; run `codex login` while it is current",
            current.name
        ))
    })
}

async fn refresh_login(db: &Arc<Database>, provider: &mut Provider) -> Result<(), ProxyError> {
    let auth = stored_auth(provider);
    let refresh_token = auth
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ProxyError::AuthError(format!(
                "Codex provider {} has no refresh token; run `codex login` while it is current",
                provider.name
            ))
        })?
        .to_string();

    {
        let state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .get(&provider.id)
            .and_then(|s| s.dead_refresh_token.as_deref())
            == Some(refresh_token.as_str())
        {
            return Err(ProxyError::AuthError(format!(
                "Codex provider {} needs a new sign-in (its refresh token was rejected); run `codex login` while it is current",
                provider.name
            )));
        }
    }

    log::info!("[codex_pool] refreshing login for provider={}", provider.id);
    let response = crate::proxy::http_client::get()
        .post(TOKEN_ENDPOINT)
        .header("Content-Type", "application/json")
        .json(&json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ProxyError::AuthError(format!("Codex token refresh failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // Only a rejection of the token itself retires it; a 5xx or a network
        // failure says nothing about whether the token is still good.
        if matches!(status.as_u16(), 400 | 401 | 403) {
            REFRESH_STATE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(provider.id.clone())
                .or_default()
                .dead_refresh_token = Some(refresh_token);
            sign_codex_out_for(provider);
        }
        return Err(ProxyError::AuthError(format!(
            "Codex token refresh for {} was refused ({status}): {}",
            provider.name,
            body.chars().take(200).collect::<String>()
        )));
    }

    let data: Value = response.json().await.map_err(|e| {
        ProxyError::AuthError(format!("Codex token refresh: unreadable reply: {e}"))
    })?;
    let new_auth = apply_refresh_response(&auth, &data).ok_or_else(|| {
        ProxyError::AuthError("Codex token refresh returned no access token".to_string())
    })?;

    if let Some(obj) = provider.settings_config.as_object_mut() {
        obj.insert("auth".to_string(), new_auth.clone());
    }
    db.save_provider("codex", provider)
        .map_err(|e| ProxyError::Internal(format!("could not store refreshed Codex login: {e}")))?;

    {
        let mut state = REFRESH_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(provider.id.clone()).or_default();
        entry.dead_refresh_token = None;
        entry.last_refresh_at = chrono::Utc::now().timestamp();
    }

    propagate_refreshed_login(db, &new_auth).await;
    log::info!("[codex_pool] login refreshed for provider={}", provider.id);
    Ok(())
}

/// The stored `auth` with the token endpoint's reply folded in. OpenAI returns
/// a new refresh token only when it rotates one; otherwise the old one stands.
fn apply_refresh_response(auth: &Value, data: &Value) -> Option<Value> {
    let access_token = data
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let mut result = auth.clone();
    let tokens = result.get_mut("tokens")?.as_object_mut()?;
    tokens.insert("access_token".to_string(), json!(access_token));
    for key in ["id_token", "refresh_token"] {
        if let Some(v) = data
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            tokens.insert(key.to_string(), json!(v));
        }
    }
    result.as_object_mut()?.insert(
        "last_refresh".to_string(),
        json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),
    );
    Some(result)
}

/// A refresh spends the old refresh token everywhere it is held. Hand the new
/// login to the two other places that may hold this account: Codex's own
/// `auth.json` (a running session reloads it on its next 401 because the
/// account matches), and the takeover backup that is restored when the proxy
/// is turned off.
async fn propagate_refreshed_login(db: &Arc<Database>, new_auth: &Value) {
    let Some(new_key) =
        codex_account::inspect(new_auth).and_then(|l| l.account_key().map(str::to_string))
    else {
        return;
    };
    let same_account = |auth: &Value| {
        codex_account::inspect(auth)
            .and_then(|l| l.account_key().map(str::to_string))
            .as_deref()
            == Some(new_key.as_str())
    };

    let live = read_live_auth();
    if same_account(&live) {
        let merged = codex_account::transplant_login(&live, new_auth);
        let path = crate::codex_config::get_codex_auth_path();
        match serde_json::to_vec_pretty(&merged)
            .map_err(|e| e.to_string())
            .and_then(|bytes| crate::config::atomic_write(&path, &bytes).map_err(|e| e.to_string()))
        {
            Ok(()) => log::info!("[codex_pool] wrote the refreshed login through to auth.json"),
            Err(e) => log::warn!("[codex_pool] could not update auth.json: {e}"),
        }
    }

    if let Ok(Some(backup)) = db.get_live_backup("codex").await {
        if let Ok(mut backup_value) = serde_json::from_str::<Value>(&backup.original_config) {
            let backup_auth = backup_value.get("auth").cloned().unwrap_or(Value::Null);
            if same_account(&backup_auth) {
                let merged = codex_account::transplant_login(&backup_auth, new_auth);
                if let Some(obj) = backup_value.as_object_mut() {
                    obj.insert("auth".to_string(), merged);
                }
                if let Ok(text) = serde_json::to_string(&backup_value) {
                    if let Err(e) = db.save_live_backup("codex", &text).await {
                        log::warn!("[codex_pool] could not update the takeover backup: {e}");
                    }
                }
            }
        }
    }
}

// ── Quota ──────────────────────────────────────────────────────────────────

/// Reads the `x-codex-*` rate-limit headers. `primary` and `secondary` are
/// positions, not durations — the account-wide family can put its 7-day window
/// in `primary` — so each window carries its own `window-minutes`. A window
/// with no duration is how the API says "not applicable" and is dropped.
pub fn parse_quota_headers(headers: &http::HeaderMap) -> Vec<QuotaWindow> {
    #[derive(Default)]
    struct Partial {
        used: Option<f64>,
        minutes: Option<i64>,
        reset_at: Option<i64>,
    }
    let mut partials: HashMap<(String, String), Partial> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();

    for (key, value) in headers {
        let Some(rest) = key.as_str().strip_prefix("x-codex-") else {
            continue;
        };
        let Ok(value) = value.to_str() else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if let Some(family) = rest.strip_suffix("-limit-name") {
            names.insert(family.to_string(), value.to_string());
            continue;
        }
        for position in ["primary", "secondary"] {
            for field in ["used-percent", "window-minutes", "reset-at"] {
                let suffix = format!("{position}-{field}");
                let family = if rest == suffix {
                    Some(String::new())
                } else {
                    rest.strip_suffix(&format!("-{suffix}")).map(str::to_string)
                };
                let Some(family) = family else { continue };
                let entry = partials.entry((family, position.to_string())).or_default();
                match field {
                    "used-percent" => entry.used = value.parse().ok(),
                    "window-minutes" => entry.minutes = value.parse().ok(),
                    _ => entry.reset_at = value.parse().ok(),
                }
            }
        }
    }

    let mut windows: Vec<QuotaWindow> = partials
        .into_iter()
        .filter_map(|((family, _), p)| {
            let minutes = p.minutes.filter(|m| *m > 0)?;
            Some(QuotaWindow {
                limit_name: if family.is_empty() {
                    None
                } else {
                    Some(names.get(&family).cloned().unwrap_or(family))
                },
                window_minutes: minutes,
                used_percent: p.used?,
                reset_at: p.reset_at.filter(|r| *r > 0),
            })
        })
        .collect();
    windows.sort_by(|a, b| {
        (a.limit_name.is_some(), &a.limit_name, a.window_minutes).cmp(&(
            b.limit_name.is_some(),
            &b.limit_name,
            b.window_minutes,
        ))
    });
    windows
}

/// Records the quota a response carried for the account that served it.
/// Codex names a model-scoped window for its model, so the model is not
/// needed to tell which windows apply.
pub fn record_quota(provider_id: &str, headers: &http::HeaderMap) {
    let windows = parse_quota_headers(headers);
    if windows.is_empty() {
        return;
    }
    account_pool::record_windows(provider_id, None, windows);
}

/// Records an outright usage-limit refusal, so the account is passed over
/// until the reset the refusal named.
pub fn record_limit_refusal(provider_id: &str, status: u16, body: Option<&str>) {
    if status != 429 {
        return;
    }
    let parsed: Value = body
        .and_then(|b| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    let error = parsed.get("error").unwrap_or(&Value::Null);
    let now = chrono::Utc::now().timestamp();
    let until = error
        .get("resets_at")
        .and_then(Value::as_i64)
        .or_else(|| {
            error
                .get("resets_in_seconds")
                .and_then(Value::as_i64)
                .map(|s| now + s)
        })
        // No reset named: keep it out briefly rather than hammering it.
        .unwrap_or(now + 300);
    account_pool::record_limited_until(provider_id, until);
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn jwt(payload: Value) -> String {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            b64.encode(r#"{"alg":"RS256"}"#),
            b64.encode(serde_json::to_vec(&payload).unwrap()),
            b64.encode("sig")
        )
    }

    fn chatgpt_auth() -> Value {
        json!({
            "tokens": {
                "id_token": jwt(json!({
                    "email": "a@x.io",
                    "https://api.openai.com/auth": { "chatgpt_account_id": "acct-a" }
                })),
                "access_token": jwt(json!({ "exp": 4_102_444_800i64 })),
                "refresh_token": "RRR",
                "account_id": "acct-a",
            },
            "last_refresh": "2026-09-01T00:00:00Z",
        })
    }

    fn provider(settings: Value) -> Provider {
        Provider::with_id("p".into(), "P".into(), settings, None)
    }

    #[test]
    fn an_account_needs_sign_in_only_while_its_refused_token_is_stored() {
        let mut signed_out = provider(json!({ "auth": chatgpt_auth() }));
        signed_out.id = "needs-sign-in-test".into();
        assert!(!needs_sign_in(&signed_out));

        REFRESH_STATE
            .lock()
            .unwrap()
            .entry(signed_out.id.clone())
            .or_default()
            .dead_refresh_token = Some("RRR".into());
        assert!(needs_sign_in(&signed_out));

        signed_out.settings_config["auth"]["tokens"]["refresh_token"] = json!("NEW");
        assert!(!needs_sign_in(&signed_out), "a new login clears it");
    }

    #[test]
    fn official_provider_is_chatgpt() {
        assert!(is_chatgpt_provider(&provider(
            json!({ "auth": chatgpt_auth(), "config": "model = \"gpt-5\"\n" })
        )));
    }

    #[test]
    fn api_key_provider_is_not_chatgpt() {
        let mut auth = chatgpt_auth();
        auth["OPENAI_API_KEY"] = json!("sk-real");
        assert!(!is_chatgpt_provider(&provider(
            json!({ "auth": auth, "config": "" })
        )));
        assert!(!is_chatgpt_provider(&provider(
            json!({ "auth": { "OPENAI_API_KEY": "sk-real" }, "config": "" })
        )));
    }

    #[test]
    fn third_party_endpoint_is_not_chatgpt() {
        let config =
            "model_provider = \"any\"\n[model_providers.any]\nbase_url = \"https://x/v1\"\n";
        assert!(!is_chatgpt_provider(&provider(
            json!({ "auth": chatgpt_auth(), "config": config })
        )));
    }

    #[test]
    fn credentials_carry_token_and_account() {
        let creds = credentials_from(&chatgpt_auth()).unwrap();
        assert_eq!(creds.account_id.as_deref(), Some("acct-a"));
        assert!(!creds.access_token.is_empty());
        assert_eq!(access_token_expiry(&chatgpt_auth()), Some(4_102_444_800));
    }

    #[test]
    fn refresh_reply_keeps_refresh_token_when_not_rotated() {
        let out =
            apply_refresh_response(&chatgpt_auth(), &json!({ "access_token": "NEW" })).unwrap();
        assert_eq!(out["tokens"]["access_token"], "NEW");
        assert_eq!(out["tokens"]["refresh_token"], "RRR");
        assert_eq!(out["tokens"]["account_id"], "acct-a");
        assert_ne!(out["last_refresh"], "2026-09-01T00:00:00Z");
    }

    #[test]
    fn refresh_reply_takes_rotated_refresh_token() {
        let out = apply_refresh_response(
            &chatgpt_auth(),
            &json!({ "access_token": "NEW", "refresh_token": "SSS", "id_token": "III" }),
        )
        .unwrap();
        assert_eq!(out["tokens"]["refresh_token"], "SSS");
        assert_eq!(out["tokens"]["id_token"], "III");
    }

    #[test]
    fn refresh_reply_without_access_token_is_rejected() {
        assert!(apply_refresh_response(&chatgpt_auth(), &json!({})).is_none());
    }

    fn login_for(email: &str, account: &str) -> Value {
        json!({
            "tokens": {
                "id_token": jwt(json!({
                    "email": email,
                    "https://api.openai.com/auth": { "chatgpt_account_id": account }
                })),
                "access_token": jwt(json!({ "exp": 4_102_444_800i64 })),
                "refresh_token": "RRR",
                "account_id": account,
            },
            "last_refresh": "2026-09-22T00:00:00Z",
        })
    }

    fn official(id: &str, auth: Value) -> Provider {
        let mut p = Provider::with_id(
            id.into(),
            id.into(),
            json!({ "auth": auth, "config": "" }),
            None,
        );
        p.category = Some("official".into());
        p
    }

    #[test]
    #[serial_test::serial]
    fn codex_is_given_the_login_of_the_account_that_answered() {
        let home = tempfile::TempDir::new().expect("temp home");
        std::env::set_var(crate::paths::ENV_TEST_HOME, home.path());
        let db = Database::memory().expect("db");
        let serving = official("b", login_for("b@x.io", "acct-b"));
        db.save_provider("codex", &serving).expect("save");
        let path = crate::codex_config::get_codex_auth_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        crate::config::write_json_file(&path, &login_for("a@x.io", "acct-a")).expect("seed");

        save_login_of_serving_account(&db, &serving);

        let live: Value = crate::config::read_json_file(&path).expect("read");
        std::env::remove_var(crate::paths::ENV_TEST_HOME);
        assert_eq!(
            codex_account::inspect(&live).and_then(|l| l.account_key().map(str::to_string)),
            Some("acct-b".to_string())
        );
    }

    #[test]
    #[serial_test::serial]
    fn a_refused_account_picked_by_hand_signs_codex_out() {
        let home = tempfile::TempDir::new().expect("temp home");
        std::env::set_var(crate::paths::ENV_TEST_HOME, home.path());
        let picked = official("picked-signout", login_for("p@x.io", "acct-p"));
        let path = crate::codex_config::get_codex_auth_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        crate::config::write_json_file(&path, &login_for("other@x.io", "acct-o")).expect("seed");
        REFRESH_STATE
            .lock()
            .unwrap()
            .entry(picked.id.clone())
            .or_default()
            .dead_refresh_token = Some("RRR".into());

        sign_codex_out_for(&picked);
        let untouched: Value = crate::config::read_json_file(&path).expect("read");
        assert!(
            untouched.get("tokens").is_some(),
            "not picked by hand: Codex keeps its login"
        );

        super::super::manual_hold::hold("codex", &picked.id);
        sign_codex_out_for(&picked);
        let signed_out = !path.exists();
        super::super::manual_hold::release("codex");
        std::env::remove_var(crate::paths::ENV_TEST_HOME);
        assert!(
            signed_out,
            "Codex is signed out as `codex logout` leaves it"
        );
    }

    #[test]
    #[serial_test::serial]
    fn a_new_sign_in_of_the_refused_account_is_kept() {
        let home = tempfile::TempDir::new().expect("temp home");
        std::env::set_var(crate::paths::ENV_TEST_HOME, home.path());
        let picked = official("picked-resigned", login_for("p@x.io", "acct-p"));
        let path = crate::codex_config::get_codex_auth_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let mut fresh = login_for("p@x.io", "acct-p");
        fresh["tokens"]["refresh_token"] = json!("NEW");
        crate::config::write_json_file(&path, &fresh).expect("seed");
        REFRESH_STATE
            .lock()
            .unwrap()
            .entry(picked.id.clone())
            .or_default()
            .dead_refresh_token = Some("RRR".into());

        super::super::manual_hold::hold("codex", &picked.id);
        sign_codex_out_for(&picked);
        let kept = path.exists();
        super::super::manual_hold::release("codex");
        std::env::remove_var(crate::paths::ENV_TEST_HOME);
        assert!(kept, "the new sign-in stays");
    }

    #[test]
    #[serial_test::serial]
    fn a_signed_out_codex_gets_the_login_of_the_account_that_answered() {
        let home = tempfile::TempDir::new().expect("temp home");
        std::env::set_var(crate::paths::ENV_TEST_HOME, home.path());
        let db = Database::memory().expect("db");
        let serving = official("b-after-signout", login_for("b@x.io", "acct-b"));
        db.save_provider("codex", &serving).expect("save");
        let path = crate::codex_config::get_codex_auth_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");

        save_login_of_serving_account(&db, &serving);

        let live: Value = crate::config::read_json_file(&path).unwrap_or(Value::Null);
        std::env::remove_var(crate::paths::ENV_TEST_HOME);
        assert_eq!(
            codex_account::inspect(&live).and_then(|l| l.account_key().map(str::to_string)),
            Some("acct-b".to_string())
        );
    }

    #[test]
    fn a_login_of_a_held_account_goes_to_its_holder() {
        let providers = vec![
            official("empty", json!({})),
            official("holder", login_for("a@x.io", "acct-a")),
        ];
        assert_eq!(
            live_login_home(&providers, Some("empty"), &login_for("a@x.io", "acct-a")),
            Some(LiveLoginHome::Holder("holder".into()))
        );
    }

    #[test]
    fn a_login_of_a_new_account_goes_to_the_current_official_provider_with_no_login() {
        let providers = vec![
            official("empty", json!({})),
            official("holder", login_for("a@x.io", "acct-a")),
        ];
        let live = login_for("b@x.io", "acct-b");
        assert_eq!(
            live_login_home(&providers, Some("empty"), &live),
            Some(LiveLoginHome::Current("empty".into()))
        );
        // The current provider already holds a usable login of another
        // account: nothing is overwritten.
        assert_eq!(live_login_home(&providers, Some("holder"), &live), None);
        // Not an Official provider: an API-key provider never takes a login.
        let mut keyed = official("keyed", json!({ "OPENAI_API_KEY": "sk-x" }));
        keyed.category = Some("third_party".into());
        assert_eq!(live_login_home(&[keyed], Some("keyed"), &live), None);
    }

    #[test]
    fn a_logged_out_live_file_goes_nowhere() {
        let providers = vec![official("empty", json!({}))];
        let mut blanked = login_for("b@x.io", "acct-b");
        blanked["tokens"]["refresh_token"] = json!("");
        assert_eq!(live_login_home(&providers, Some("empty"), &blanked), None);
        assert_eq!(
            live_login_home(&providers, Some("empty"), &Value::Null),
            None
        );
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
    fn quota_headers_are_read_by_duration_not_position() {
        let windows = parse_quota_headers(&headers(&[
            ("x-codex-primary-used-percent", "42"),
            ("x-codex-primary-window-minutes", "10080"),
            ("x-codex-primary-reset-at", "1900000000"),
            ("x-codex-secondary-used-percent", "0"),
            ("x-codex-secondary-window-minutes", "0"),
            ("x-codex-spark-primary-used-percent", "7"),
            ("x-codex-spark-primary-window-minutes", "300"),
            ("x-codex-spark-limit-name", "GPT-5.3-Codex-Spark"),
        ]));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].limit_name, None);
        assert_eq!(windows[0].window_minutes, 10080);
        assert_eq!(windows[0].used_percent, 42.0);
        assert_eq!(windows[0].reset_at, Some(1_900_000_000));
        assert_eq!(
            windows[1].limit_name.as_deref(),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(windows[1].window_minutes, 300);
    }
}
