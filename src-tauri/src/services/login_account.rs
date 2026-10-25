//! Cursor CLI and Antigravity CLI accounts: which account a stored login
//! belongs to, the switch-away backfill guard, and each account's usage.
//!
//! A provider of these apps is only a subscription login, written to where
//! the CLI reads it on Enable (`cursor_config`, `antigravity_config`). The
//! switch-away backfill files the live login back into the provider being
//! left under the rules Codex and Claude accounts follow:
//!
//!   * a blank or removed login never replaces a usable stored one,
//!   * an older login never replaces a newer stored one,
//!   * a login of a *different* account is never filed under the outgoing
//!     provider; it goes to the provider that already holds that account,
//!     unless that one's copy is newer.

use serde_json::Value;

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::services::subscription::{CredentialStatus, QuotaTier, SubscriptionQuota};

/// The settings keys that make up an app's login.
fn login_keys(app: &AppType) -> &'static [&'static str] {
    match app {
        AppType::Cursor => &["auth", "authInfo"],
        AppType::Antigravity => &["credential"],
        _ => &[],
    }
}

/// What a stored login says about its account.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Login {
    /// Cursor: the token's `sub`; Antigravity: the signed-in email.
    pub account: Option<String>,
    pub email: Option<String>,
    /// Orders two logins of one account: Cursor's token expiry, Antigravity's
    /// access-token expiry (moved on every renewal). Unix seconds, 0 unknown.
    pub freshness: i64,
    /// Both tokens are present.
    pub alive: bool,
}

fn nonempty(obj: Option<&Value>, key: &str) -> bool {
    obj.and_then(|o| o.get(key))
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
}

/// Reads the login out of a provider's settings; `None` when it holds none.
pub fn inspect(app: &AppType, settings: &Value) -> Option<Login> {
    match app {
        AppType::Cursor => {
            let auth = settings.get("auth").filter(|v| v.is_object())?;
            let claims = auth
                .get("accessToken")
                .and_then(Value::as_str)
                .and_then(crate::services::codex_account::decode_jwt_payload);
            let info = settings.get("authInfo");
            Some(Login {
                account: claims
                    .as_ref()
                    .and_then(|c| c.get("sub"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                email: info
                    .and_then(|i| i.get("email"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                freshness: claims
                    .as_ref()
                    .and_then(|c| c.get("exp"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                alive: nonempty(Some(auth), "accessToken") && nonempty(Some(auth), "refreshToken"),
            })
        }
        AppType::Antigravity => {
            let credential = settings.get("credential").filter(|v| v.is_object())?;
            let token = credential.get("token");
            let email = credential
                .get("id_token_email")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(Login {
                account: email.clone(),
                email,
                freshness: token
                    .and_then(|t| t.get("expiry"))
                    .and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp())
                    .unwrap_or(0),
                alive: nonempty(token, "access_token") && nonempty(token, "refresh_token"),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackfillVerdict {
    Accept,
    KeepStored(&'static str),
    /// The live login is another account, named here.
    ForeignAccount(String),
}

/// Pure decision over the outgoing provider's stored settings and the live ones.
pub fn judge_backfill(app: &AppType, stored: &Value, live: &Value) -> BackfillVerdict {
    let Some(stored_login) = inspect(app, stored).filter(|l| l.alive) else {
        return BackfillVerdict::Accept;
    };
    let Some(live_login) = inspect(app, live) else {
        return BackfillVerdict::KeepStored("live login removed");
    };
    if !live_login.alive {
        return BackfillVerdict::KeepStored("live login blanked");
    }
    if let (Some(a), Some(b)) = (&stored_login.account, &live_login.account) {
        if a != b {
            return BackfillVerdict::ForeignAccount(b.clone());
        }
    }
    if live_login.freshness < stored_login.freshness {
        return BackfillVerdict::KeepStored("live login older than stored");
    }
    BackfillVerdict::Accept
}

fn copy_login(app: &AppType, onto: &mut Provider, from: &Value) {
    let Some(obj) = onto.settings_config.as_object_mut() else {
        return;
    };
    for key in login_keys(app) {
        obj.insert(
            key.to_string(),
            from.get(*key).cloned().unwrap_or(Value::Null),
        );
    }
}

/// Outcome of [`reconcile_switch_away`], for the switch result's warnings.
#[derive(Debug, Default)]
pub struct SwitchAwayOutcome {
    pub warnings: Vec<String>,
    /// Another provider that receives the live login because it holds that account.
    pub rehomed: Option<Provider>,
}

/// Applies the backfill rules to the outgoing provider, which already carries
/// the live settings; `stored` is what it held before.
pub fn reconcile_switch_away(
    app: &AppType,
    stored: &Provider,
    outgoing: &mut Provider,
    others: &indexmap::IndexMap<String, Provider>,
) -> SwitchAwayOutcome {
    let mut outcome = SwitchAwayOutcome::default();
    let live = outgoing.settings_config.clone();
    let tag = app.as_str();
    match judge_backfill(app, &stored.settings_config, &live) {
        BackfillVerdict::Accept => {}
        BackfillVerdict::KeepStored(reason) => {
            log::info!(
                "[{tag}_account] backfill keeps stored login for provider={} ({reason})",
                outgoing.id
            );
            copy_login(app, outgoing, &stored.settings_config);
        }
        BackfillVerdict::ForeignAccount(account) => {
            log::warn!(
                "[{tag}_account] live login ({account}) is not the account provider={} holds; refusing to file it there",
                outgoing.id
            );
            copy_login(app, outgoing, &stored.settings_config);
            outcome
                .warnings
                .push(format!("backfill_account_mismatch:{}", outgoing.id));
            let live_login = inspect(app, &live).unwrap_or_default();
            let owner = others.values().find(|p| {
                p.id != outgoing.id
                    && inspect(app, &p.settings_config).and_then(|l| l.account)
                        == Some(account.clone())
            });
            let Some(owner) = owner else {
                log::warn!(
                    "[{tag}_account] no provider holds account {account}; its login is not kept"
                );
                return outcome;
            };
            let owner_login = inspect(app, &owner.settings_config).unwrap_or_default();
            if owner_login.alive && owner_login.freshness > live_login.freshness {
                return outcome;
            }
            let mut rehomed = owner.clone();
            copy_login(app, &mut rehomed, &live);
            log::info!(
                "[{tag}_account] re-homed live login for {account} to provider={}",
                rehomed.id
            );
            outcome.rehomed = Some(rehomed);
        }
    }
    outcome
}

/// The account a card names.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginIdentity {
    pub email: Option<String>,
    pub account: Option<String>,
}

fn read_live(app: &AppType) -> Option<Value> {
    let live = match app {
        AppType::Cursor => crate::cursor_config::read_cursor_live(),
        AppType::Antigravity => crate::antigravity_config::read_antigravity_live(),
        _ => return None,
    };
    match live {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!(
                "[{}_account] could not read the live login: {e}",
                app.as_str()
            );
            None
        }
    }
}

/// The login a provider's card shows and reads usage from. The current
/// provider's is the live one when it is the same account (or the provider
/// holds none yet), and that live login is filed into the provider under the
/// backfill rules, so a sign-in made while the card is current is kept with
/// it without a switch.
fn effective_login(
    state: &crate::store::AppState,
    app: &AppType,
    provider: &Provider,
) -> Result<Value, crate::error::AppError> {
    let is_current = crate::settings::get_effective_current_provider(&state.db, app)?.as_deref()
        == Some(provider.id.as_str());
    if !is_current {
        return Ok(provider.settings_config.clone());
    }
    let Some(live) = read_live(app) else {
        return Ok(provider.settings_config.clone());
    };
    if judge_backfill(app, &provider.settings_config, &live) != BackfillVerdict::Accept {
        return Ok(provider.settings_config.clone());
    }
    let mut updated = provider.clone();
    copy_login(app, &mut updated, &live);
    if updated.settings_config != provider.settings_config && inspect(app, &live).is_some() {
        match state.db.save_provider(app.as_str(), &updated) {
            Ok(()) => log::info!(
                "[{}_account] filed live login into provider={}",
                app.as_str(),
                provider.id
            ),
            Err(e) => log::warn!(
                "[{}_account] could not file live login into provider={}: {e}",
                app.as_str(),
                provider.id
            ),
        }
    }
    Ok(updated.settings_config)
}

pub fn read_identity(
    state: &crate::store::AppState,
    app: &AppType,
    provider_id: &str,
) -> Result<Option<LoginIdentity>, crate::error::AppError> {
    let Some(provider) = state.db.get_provider_by_id(provider_id, app.as_str())? else {
        return Ok(None);
    };
    let settings = effective_login(state, app, &provider)?;
    Ok(inspect(app, &settings).map(|l| LoginIdentity {
        email: l.email,
        account: l.account,
    }))
}

// ── Usage ─────────────────────────────────────────────────

fn quota(app: &AppType, tiers: Vec<QuotaTier>) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: app.as_str().to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(chrono::Utc::now().timestamp_millis()),
    }
}

fn failure(app: &AppType, status: CredentialStatus, message: String) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: app.as_str().to_string(),
        credential_status: status,
        credential_message: Some(message.clone()),
        success: false,
        tiers: vec![],
        extra_usage: None,
        error: Some(message),
        queried_at: Some(chrono::Utc::now().timestamp_millis()),
    }
}

fn ms_to_iso(ms: &Value) -> Option<String> {
    let ms = ms
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| ms.as_i64())?;
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339())
}

/// Cursor's included usage for the billing month, as `cursor-agent` shows it
/// (Included, Auto, API).
pub(crate) fn cursor_tiers(body: &Value) -> Vec<QuotaTier> {
    let usage = &body["planUsage"];
    let resets_at = ms_to_iso(&body["billingCycleEnd"]);
    let total = usage["totalPercentUsed"].as_f64().or_else(|| {
        let limit = usage["limit"].as_f64().filter(|l| *l > 0.0)?;
        Some(usage["includedSpend"].as_f64()? / limit * 100.0)
    });
    [
        ("cursor_included", total),
        ("cursor_auto", usage["autoPercentUsed"].as_f64()),
        ("cursor_api", usage["apiPercentUsed"].as_f64()),
    ]
    .into_iter()
    .filter_map(|(name, used)| {
        Some(QuotaTier {
            name: name.to_string(),
            utilization: used?.clamp(0.0, 100.0),
            resets_at: resets_at.clone(),
        })
    })
    .collect()
}

async fn query_cursor(settings: &Value) -> SubscriptionQuota {
    let app = AppType::Cursor;
    let Some(token) = settings["auth"]["accessToken"].as_str() else {
        return SubscriptionQuota::not_found("cursor");
    };
    let resp = crate::proxy::http_client::get()
        .post("https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage")
        .bearer_auth(token)
        .header("Connect-Protocol-Version", "1")
        .json(&serde_json::json!({}))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return failure(&app, CredentialStatus::Valid, format!("Network error: {e}")),
    };
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return failure(
            &app,
            CredentialStatus::SignedOut,
            "The login was refused; sign this account in again".to_string(),
        );
    }
    if !status.is_success() {
        return failure(
            &app,
            CredentialStatus::Valid,
            format!("Cursor usage failed (HTTP {status})"),
        );
    }
    match resp.json::<Value>().await {
        Ok(body) => quota(&app, cursor_tiers(&body)),
        Err(e) => failure(
            &app,
            CredentialStatus::Valid,
            format!("Could not parse Cursor usage: {e}"),
        ),
    }
}

/// The service refuses this endpoint (403) to a client whose user agent does
/// not begin with `antigravity`; the version after it is not checked.
const ANTIGRAVITY_USER_AGENT: &str = "antigravity";

/// The installed `agy` executable.
fn agy_binary() -> Option<std::path::PathBuf> {
    let installed = std::env::var_os("LOCALAPPDATA").map(|dir| {
        std::path::PathBuf::from(dir)
            .join("agy")
            .join("bin")
            .join("agy.exe")
    });
    if let Some(path) = installed.filter(|p| p.is_file()) {
        return Some(path);
    }
    let exe = if cfg!(windows) { "agy.exe" } else { "agy" };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(exe))
            .find(|p| p.is_file())
    })
}

/// Every OAuth client id and secret in `agy`'s executable. A stored login's
/// one-hour access token is renewed with `agy`'s own client, which is read
/// from the installed binary rather than kept in Switchy's source. The file
/// is about 200 MB, so it is scanned in chunks, once per run.
fn agy_oauth_clients() -> &'static [(String, String)] {
    static CLIENTS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    CLIENTS.get_or_init(|| {
        use std::io::Read;
        let ids_re =
            regex::bytes::Regex::new(r"[0-9]{6,}-[a-z0-9]{32}\.apps\.googleusercontent\.com")
                .expect("valid regex");
        let secrets_re =
            regex::bytes::Regex::new(r"GOCSPX-[A-Za-z0-9_-]{28}").expect("valid regex");
        let mut ids = std::collections::BTreeSet::new();
        let mut secrets = std::collections::BTreeSet::new();
        let Some(mut file) = agy_binary().and_then(|p| std::fs::File::open(p).ok()) else {
            return Vec::new();
        };
        const CHUNK: usize = 8 << 20;
        const OVERLAP: usize = 128;
        let mut buf = vec![0u8; CHUNK + OVERLAP];
        let mut carried = 0;
        loop {
            let n = match file.read(&mut buf[carried..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let window = &buf[..carried + n];
            for m in ids_re.find_iter(window) {
                ids.insert(String::from_utf8_lossy(m.as_bytes()).into_owned());
            }
            for m in secrets_re.find_iter(window) {
                secrets.insert(String::from_utf8_lossy(m.as_bytes()).into_owned());
            }
            carried = window.len().min(OVERLAP);
            let from = window.len() - carried;
            buf.copy_within(from..from + carried, 0);
        }
        ids.iter()
            .flat_map(|id| {
                secrets
                    .iter()
                    .map(move |secret| (id.clone(), secret.clone()))
            })
            .collect()
    })
}

/// The `agy` client that last renewed a login, tried first next time.
static AGY_CLIENT: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// Access tokens renewed for usage reads, by refresh token, until they expire.
static RENEWED: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, (String, i64)>>,
> = once_cell::sync::Lazy::new(Default::default);

enum Renewal {
    Token(String),
    Refused,
    Failed(String),
}

async fn antigravity_access_token(credential: &Value) -> Renewal {
    let token = &credential["token"];
    let now = chrono::Utc::now().timestamp();
    let expiry = token["expiry"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    if let Some(access) = token["access_token"].as_str() {
        if expiry > now + 60 {
            return Renewal::Token(access.to_string());
        }
    }
    let Some(refresh) = token["refresh_token"].as_str() else {
        return Renewal::Refused;
    };
    if let Some((access, until)) = RENEWED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(refresh)
        .cloned()
    {
        if until > now + 60 {
            return Renewal::Token(access);
        }
    }
    let clients = tokio::task::spawn_blocking(agy_oauth_clients)
        .await
        .unwrap_or(&[]);
    if clients.is_empty() {
        return Renewal::Failed(
            "agy is not installed here, so its login cannot be renewed".to_string(),
        );
    }
    let remembered = AGY_CLIENT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let ordered = remembered
        .iter()
        .chain(clients.iter().filter(|c| Some(*c) != remembered.as_ref()));
    for (client_id, client_secret) in ordered {
        let resp = crate::proxy::http_client::get()
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("refresh_token", refresh),
                ("grant_type", "refresh_token"),
            ])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => return Renewal::Failed(format!("Network error renewing the login: {e}")),
        };
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            match body["error"].as_str() {
                // Another of agy's clients issued this login.
                Some("invalid_client" | "unauthorized_client") => continue,
                Some("invalid_grant") => return Renewal::Refused,
                _ => return Renewal::Failed(format!("Renewing the login failed (HTTP {status})")),
            }
        }
        let Some(access) = body["access_token"].as_str() else {
            return Renewal::Failed("The renewal carried no access token".to_string());
        };
        *AGY_CLIENT.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((client_id.clone(), client_secret.clone()));
        let until = now + body["expires_in"].as_i64().unwrap_or(3600);
        RENEWED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(refresh.to_string(), (access.to_string(), until));
        return Renewal::Token(access.to_string());
    }
    Renewal::Failed("None of agy's OAuth clients renewed this login".to_string())
}

/// Antigravity's quota per model family: the lowest remaining share among a
/// provider's models (Gemini, Claude, GPT-OSS) and when it resets. Internal
/// models with no reset time are left out.
pub(crate) fn antigravity_tiers(body: &Value) -> Vec<QuotaTier> {
    let mut families: indexmap::IndexMap<&'static str, (f64, Option<String>)> =
        indexmap::IndexMap::new();
    let Some(models) = body["models"].as_object() else {
        return vec![];
    };
    for model in models.values() {
        let info = &model["quotaInfo"];
        let Some(reset) = info["resetTime"].as_str() else {
            continue;
        };
        let family = match model["modelProvider"].as_str().unwrap_or("") {
            "MODEL_PROVIDER_GOOGLE" => "antigravity_gemini",
            "MODEL_PROVIDER_ANTHROPIC" => "antigravity_claude",
            "MODEL_PROVIDER_OPENAI" => "antigravity_gpt",
            _ => continue,
        };
        let remaining = info["remainingFraction"]
            .as_f64()
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let entry = families
            .entry(family)
            .or_insert((remaining, Some(reset.to_string())));
        if remaining < entry.0 {
            *entry = (remaining, Some(reset.to_string()));
        }
    }
    let order = [
        "antigravity_gemini",
        "antigravity_claude",
        "antigravity_gpt",
    ];
    let mut tiers: Vec<QuotaTier> = families
        .into_iter()
        .map(|(name, (remaining, resets_at))| QuotaTier {
            name: name.to_string(),
            utilization: (1.0 - remaining) * 100.0,
            resets_at,
        })
        .collect();
    tiers.sort_by_key(|t| order.iter().position(|o| *o == t.name));
    tiers
}

async fn query_antigravity(settings: &Value) -> SubscriptionQuota {
    let app = AppType::Antigravity;
    let credential = &settings["credential"];
    if !credential.is_object() {
        return SubscriptionQuota::not_found("antigravity");
    }
    let token = match antigravity_access_token(credential).await {
        Renewal::Token(t) => t,
        Renewal::Refused => {
            return failure(
                &app,
                CredentialStatus::SignedOut,
                "The login was refused; sign this account in again".to_string(),
            )
        }
        Renewal::Failed(message) => return failure(&app, CredentialStatus::Valid, message),
    };
    let resp = crate::proxy::http_client::get()
        .post("https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels")
        .bearer_auth(&token)
        .header("User-Agent", ANTIGRAVITY_USER_AGENT)
        .json(&serde_json::json!({}))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return failure(&app, CredentialStatus::Valid, format!("Network error: {e}")),
    };
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return failure(
            &app,
            CredentialStatus::Expired,
            "The access token was refused".to_string(),
        );
    }
    if !status.is_success() {
        return failure(
            &app,
            CredentialStatus::Valid,
            format!("Antigravity quota failed (HTTP {status})"),
        );
    }
    match resp.json::<Value>().await {
        Ok(body) => quota(&app, antigravity_tiers(&body)),
        Err(e) => failure(
            &app,
            CredentialStatus::Valid,
            format!("Could not parse Antigravity quota: {e}"),
        ),
    }
}

/// Usage of a provider's own account.
pub async fn quota_for_provider(
    state: &crate::store::AppState,
    app: &AppType,
    provider_id: &str,
) -> Result<SubscriptionQuota, String> {
    let provider = state
        .db
        .get_provider_by_id(provider_id, app.as_str())
        .map_err(|e| e.to_string())?;
    let Some(provider) = provider else {
        return Ok(SubscriptionQuota::not_found(app.as_str()));
    };
    let settings = effective_login(state, app, &provider).map_err(|e| e.to_string())?;
    Ok(match app {
        AppType::Cursor => query_cursor(&settings).await,
        AppType::Antigravity => query_antigravity(&settings).await,
        _ => SubscriptionQuota::not_found(app.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn cursor(sub: &str, email: &str, exp: i64) -> Value {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let token = format!(
            "{}.{}.sig",
            b64.encode(r#"{"alg":"HS256"}"#),
            b64.encode(json!({ "sub": sub, "exp": exp }).to_string())
        );
        json!({
            "auth": { "accessToken": token, "refreshToken": token },
            "authInfo": { "email": email, "userId": 1 }
        })
    }

    fn agy(email: &str, refresh: &str, expiry: &str) -> Value {
        json!({ "credential": {
            "token": { "access_token": "ya29.x", "refresh_token": refresh, "expiry": expiry },
            "auth_method": "consumer",
            "id_token_email": email
        }})
    }

    fn provider(id: &str, settings: Value) -> Provider {
        Provider::with_id(id.into(), id.into(), settings, None)
    }

    #[test]
    fn a_cursor_login_names_its_account_from_the_token_and_authinfo() {
        let login = inspect(&AppType::Cursor, &cursor("auth0|u1", "a@x.io", 100)).unwrap();
        assert_eq!(login.account.as_deref(), Some("auth0|u1"));
        assert_eq!(login.email.as_deref(), Some("a@x.io"));
        assert_eq!(login.freshness, 100);
        assert!(login.alive);
        assert!(inspect(&AppType::Cursor, &json!({ "auth": null })).is_none());
    }

    #[test]
    fn the_backfill_rules_hold_for_both_apps() {
        let c = AppType::Cursor;
        let stored = cursor("u1", "a@x.io", 100);
        assert_eq!(
            judge_backfill(&c, &json!({}), &stored),
            BackfillVerdict::Accept
        );
        assert_eq!(
            judge_backfill(&c, &stored, &cursor("u1", "a@x.io", 200)),
            BackfillVerdict::Accept
        );
        assert!(matches!(
            judge_backfill(&c, &stored, &json!({ "auth": null, "authInfo": null })),
            BackfillVerdict::KeepStored(_)
        ));
        assert!(matches!(
            judge_backfill(&c, &stored, &cursor("u1", "a@x.io", 50)),
            BackfillVerdict::KeepStored(_)
        ));
        assert_eq!(
            judge_backfill(&c, &stored, &cursor("u2", "b@x.io", 200)),
            BackfillVerdict::ForeignAccount("u2".into())
        );

        let a = AppType::Antigravity;
        let stored = agy("a@x.io", "1//R", "2026-10-24T17:00:00-07:00");
        assert_eq!(
            judge_backfill(
                &a,
                &stored,
                &agy("a@x.io", "1//R", "2026-10-24T18:00:00-07:00")
            ),
            BackfillVerdict::Accept
        );
        assert!(matches!(
            judge_backfill(&a, &stored, &agy("a@x.io", "", "2026-10-24T18:00:00-07:00")),
            BackfillVerdict::KeepStored(_)
        ));
        assert_eq!(
            judge_backfill(
                &a,
                &stored,
                &agy("b@x.io", "1//S", "2026-10-24T18:00:00-07:00")
            ),
            BackfillVerdict::ForeignAccount("b@x.io".into())
        );
    }

    #[test]
    fn a_foreign_login_goes_to_the_provider_holding_that_account() {
        let app = AppType::Cursor;
        let stored = provider("A", cursor("u1", "a@x.io", 100));
        let mut outgoing = provider("A", cursor("u2", "b@x.io", 300));
        let mut others = indexmap::IndexMap::new();
        others.insert("A".to_string(), stored.clone());
        others.insert("B".to_string(), provider("B", cursor("u2", "b@x.io", 200)));

        let outcome = reconcile_switch_away(&app, &stored, &mut outgoing, &others);
        assert_eq!(outgoing.settings_config, stored.settings_config);
        assert_eq!(outcome.warnings, vec!["backfill_account_mismatch:A"]);
        let rehomed = outcome.rehomed.expect("B receives the login");
        assert_eq!(rehomed.id, "B");
        assert_eq!(
            inspect(&app, &rehomed.settings_config).unwrap().freshness,
            300
        );
    }

    #[test]
    fn cursor_usage_reads_the_three_meters() {
        let tiers = cursor_tiers(&json!({
            "billingCycleEnd": "1793289350000",
            "planUsage": { "includedSpend": 1024, "limit": 2000, "autoPercentUsed": 2.5, "apiPercentUsed": 0 }
        }));
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "cursor_included");
        assert!((tiers[0].utilization - 51.2).abs() < 1e-9);
        assert!(tiers[0]
            .resets_at
            .as_deref()
            .unwrap()
            .starts_with("2026-10-29"));
    }

    #[test]
    fn antigravity_usage_takes_the_lowest_share_per_family() {
        let tiers = antigravity_tiers(&json!({ "models": {
            "g1": { "modelProvider": "MODEL_PROVIDER_GOOGLE", "quotaInfo": { "remainingFraction": 0.8, "resetTime": "R1" } },
            "g2": { "modelProvider": "MODEL_PROVIDER_GOOGLE", "quotaInfo": { "remainingFraction": 0.25, "resetTime": "R2" } },
            "tab": { "modelProvider": "MODEL_PROVIDER_GOOGLE", "quotaInfo": { "remainingFraction": 0.0 } },
            "c1": { "modelProvider": "MODEL_PROVIDER_ANTHROPIC", "quotaInfo": { "remainingFraction": 1, "resetTime": "R3" } }
        }}));
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "antigravity_gemini");
        assert!((tiers[0].utilization - 75.0).abs() < 1e-9);
        assert_eq!(tiers[0].resets_at.as_deref(), Some("R2"));
        assert_eq!(tiers[1].name, "antigravity_claude");
    }
}
