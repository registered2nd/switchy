//! Signing a pooled Codex or Claude account in while every program stays on
//! the account it is using.
//!
//! Switchy runs the provider's own OAuth flow, with the parameters the
//! official CLI sends: it builds the authorize link (PKCE S256, a random
//! state), the caller opens it in that account's own browser profile and
//! approves, and Switchy exchanges the code itself, through the card's own
//! proxy, the route its token renewals take. The new login is checked to be
//! the card's account and filed with the card; nothing is switched and no
//! CLI login runs, so the live `~/.codex/auth.json`, `~/.claude/.credentials.json`
//! and their WSL mirrors are left as they are.
//!
//! Codex's link returns to `http://127.0.0.1:1455/auth/callback`, which
//! Switchy answers while a Codex sign-in is pending; Claude's page shows a
//! code (`<code>#<state>`) to hand to `sign_in_finish`. The verifier and state
//! live only in memory.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::provider::Provider;
use crate::proxy::{claude_pool, codex_pool};
use crate::services::codex_account;
use crate::store::AppState;

/// A pending sign-in is dropped this long after it started.
pub const SIGN_IN_TTL_SECS: i64 = 15 * 60;
/// How long the outcome of a sign-in stays readable with `sign_in_status`.
const OUTCOME_TTL_SECS: i64 = 3600;

// Codex: codex-rs/login/src/server.rs (`build_authorize_url`,
// `exchange_code_for_tokens`, `persist_tokens_async`) and oauth/ of the
// installed CLI's release.
const CODEX_ISSUER: &str = "https://auth.openai.com";
const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
/// The callback port `codex login` listens on, then its registered fallback.
const CODEX_CALLBACK_PORTS: [u16; 2] = [1455, 1457];

// Claude: Claude Code's `/login` "Claude account" flow, read from its bundle.
const CLAUDE_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const CLAUDE_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload user:plugins";
const CLAUDE_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const CLAUDE_PROFILE_USER_AGENT: &str = "claude-code/2.1.310";
/// Claude Code's lifetime for a refresh token the reply gives none for.
const CLAUDE_REFRESH_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInApp {
    Codex,
    Claude,
}

impl SignInApp {
    pub fn parse(app: &str) -> Result<Self, String> {
        match app {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            other => Err(format!(
                "{other} has no sign-in Switchy can run; use codex or claude"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInStart {
    pub session_id: String,
    pub url: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// `callback`: the browser returns to Switchy and the sign-in finishes by
    /// itself. `code`: hand the code (or the address the browser landed on)
    /// to `sign_in_finish`.
    pub finish: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SignedIn {
    pub email: String,
    pub account_id: String,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SignInStatus {
    /// `pending`, `done`, `failed` or `expired`.
    pub status: &'static str,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

// ── PKCE and the authorize link ─────────────────────────────────────────────

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `n` random bytes from the OS generator (each v4 UUID carries 122 random
/// bits), base64url-encoded as both CLIs encode their verifier and state.
fn random_b64url(n: usize) -> String {
    let mut bytes = Vec::with_capacity(n + 16);
    while bytes.len() < n {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(n);
    b64url(&bytes)
}

/// The S256 challenge: base64url(SHA-256(verifier)), no padding.
pub fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn codex_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/auth/callback")
}

/// The authorize link, with the parameters in the order the CLI sends them.
pub fn authorize_url(app: SignInApp, challenge: &str, state: &str, redirect_uri: &str) -> String {
    let (base, pairs): (&str, Vec<(&str, &str)>) = match app {
        SignInApp::Codex => (
            CODEX_AUTHORIZE_URL,
            vec![
                ("response_type", "code"),
                ("client_id", codex_pool::CLIENT_ID),
                ("redirect_uri", redirect_uri),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("state", state),
                ("scope", CODEX_SCOPE),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("originator", CODEX_ORIGINATOR),
            ],
        ),
        SignInApp::Claude => (
            CLAUDE_AUTHORIZE_URL,
            vec![
                ("code", "true"),
                ("client_id", claude_pool::CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", redirect_uri),
                ("scope", CLAUDE_SCOPE),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("state", state),
            ],
        ),
    };
    let mut url = url::Url::parse(base).expect("authorize URL");
    url.query_pairs_mut().extend_pairs(pairs);
    url.to_string()
}

// ── Pending sign-ins ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Pending {
    session_id: String,
    app: SignInApp,
    provider_id: String,
    verifier: String,
    state: String,
    redirect_uri: String,
    expires_at: i64,
    /// A finish is exchanging this sign-in's code now.
    finishing: bool,
}

#[derive(Debug, Clone)]
enum Outcome {
    Done(SignedIn),
    Failed(String),
    Expired,
}

#[derive(Debug, Clone)]
struct Finished {
    provider_id: String,
    outcome: Outcome,
    at: i64,
}

#[derive(Default)]
struct Sessions {
    pending: Vec<Pending>,
    finished: HashMap<String, Finished>,
}

impl Sessions {
    fn settle(&mut self, session_id: &str, provider_id: &str, outcome: Outcome, now: i64) {
        self.pending.retain(|p| p.session_id != session_id);
        self.finished.insert(
            session_id.to_string(),
            Finished {
                provider_id: provider_id.to_string(),
                outcome,
                at: now,
            },
        );
    }

    /// Drops what has run out: pending sign-ins past their expiry (not one
    /// being finished) and outcomes kept long enough.
    fn expire(&mut self, now: i64) {
        let due: Vec<(String, String)> = self
            .pending
            .iter()
            .filter(|p| p.expires_at <= now && !p.finishing)
            .map(|p| (p.session_id.clone(), p.provider_id.clone()))
            .collect();
        for (session_id, provider_id) in due {
            self.settle(&session_id, &provider_id, Outcome::Expired, now);
        }
        self.finished.retain(|_, f| now - f.at < OUTCOME_TTL_SECS);
    }

    /// Adds a sign-in, replacing any pending one of the same card.
    fn begin(&mut self, pending: Pending, now: i64) {
        self.expire(now);
        let replaced: Vec<String> = self
            .pending
            .iter()
            .filter(|p| p.provider_id == pending.provider_id && p.app == pending.app)
            .map(|p| p.session_id.clone())
            .collect();
        for session_id in replaced {
            self.settle(
                &session_id,
                &pending.provider_id,
                Outcome::Failed("replaced by a newer sign-in of this card".to_string()),
                now,
            );
        }
        self.pending.push(pending);
    }

    /// Takes a pending sign-in for finishing, found by `matches`.
    fn claim(
        &mut self,
        now: i64,
        matches: impl Fn(&Pending) -> bool,
        unknown: impl FnOnce(&Self) -> String,
    ) -> Result<Pending, String> {
        self.expire(now);
        match self.pending.iter_mut().find(|p| matches(p)) {
            Some(p) if p.finishing => Err("this sign-in is already being finished".to_string()),
            Some(p) => {
                p.finishing = true;
                Ok(p.clone())
            }
            None => Err(unknown(self)),
        }
    }

    fn claim_session(&mut self, session_id: &str, now: i64) -> Result<Pending, String> {
        self.claim(
            now,
            |p| p.session_id == session_id,
            |sessions| match sessions.finished.get(session_id).map(|f| &f.outcome) {
                Some(Outcome::Expired) => {
                    "this sign-in expired; start it again with sign_in_start".to_string()
                }
                Some(Outcome::Done(_)) => "this sign-in is already finished".to_string(),
                Some(Outcome::Failed(message)) => format!("this sign-in already ended: {message}"),
                None => format!("no sign-in {session_id}"),
            },
        )
    }

    fn claim_state(&mut self, state: &str, now: i64) -> Result<Pending, String> {
        self.claim(
            now,
            |p| p.app == SignInApp::Codex && state_matches(state, &p.state),
            |_| "no sign-in is waiting for this link: it finished, expired or was replaced. Start it again.".to_string(),
        )
    }

    /// Hands a claimed sign-in back, after input that was not its code.
    fn release(&mut self, session_id: &str) {
        if let Some(p) = self.pending.iter_mut().find(|p| p.session_id == session_id) {
            p.finishing = false;
        }
    }

    fn status(&mut self, session_id: &str, now: i64) -> Option<SignInStatus> {
        self.expire(now);
        let blank = |status, provider_id: &str| SignInStatus {
            status,
            provider_id: provider_id.to_string(),
            email: None,
            account_id: None,
            message: None,
            expires_at: None,
        };
        if let Some(p) = self.pending.iter().find(|p| p.session_id == session_id) {
            return Some(SignInStatus {
                expires_at: Some(p.expires_at),
                ..blank("pending", &p.provider_id)
            });
        }
        let finished = self.finished.get(session_id)?;
        Some(match &finished.outcome {
            Outcome::Done(signed) => SignInStatus {
                email: Some(signed.email.clone()),
                account_id: Some(signed.account_id.clone()),
                ..blank("done", &finished.provider_id)
            },
            Outcome::Failed(message) => SignInStatus {
                message: Some(message.clone()),
                ..blank("failed", &finished.provider_id)
            },
            Outcome::Expired => blank("expired", &finished.provider_id),
        })
    }

    fn codex_pending(&self) -> bool {
        self.pending.iter().any(|p| p.app == SignInApp::Codex)
    }
}

static SESSIONS: Lazy<Mutex<Sessions>> = Lazy::new(|| Mutex::new(Sessions::default()));

fn sessions() -> std::sync::MutexGuard<'static, Sessions> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// ChatGPT may append onboarding metadata to the state it hands back.
fn state_matches(given: &str, expected: &str) -> bool {
    !expected.is_empty() && given.starts_with(expected)
}

// ── Which cards can be signed in ────────────────────────────────────────────

/// An Official card of `app` that holds a subscription sign-in: for Codex,
/// one with no API key of its own and no endpoint of its own.
fn is_sign_in_card(app: SignInApp, provider: &Provider) -> bool {
    if provider.category.as_deref() != Some("official") {
        return false;
    }
    match app {
        SignInApp::Claude => true,
        SignInApp::Codex => {
            let settings = &provider.settings_config;
            let has_api_key = settings
                .get("auth")
                .and_then(|a| a.get("OPENAI_API_KEY"))
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|k| !k.is_empty() && k != "PROXY_MANAGED");
            let has_own_endpoint = settings
                .get("config")
                .and_then(Value::as_str)
                .is_some_and(|toml| toml.contains("base_url"));
            !has_api_key && !has_own_endpoint
        }
    }
}

fn sign_in_card(
    app_handle: &AppHandle,
    app: SignInApp,
    provider_id: &str,
) -> Result<Provider, String> {
    let state = app_handle.state::<AppState>();
    let provider = state
        .db
        .get_provider_by_id(provider_id, app.as_str())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no {} card {provider_id}", app.name()))?;
    if !is_sign_in_card(app, &provider) {
        return Err(format!(
            "{} is not an Official sign-in card of {}",
            provider.name,
            app.name()
        ));
    }
    Ok(provider)
}

// ── Start, finish, status ───────────────────────────────────────────────────

/// Starts a sign-in of `provider_id`'s account and answers the link to open.
pub fn start(
    app_handle: &AppHandle,
    app: SignInApp,
    provider_id: &str,
) -> Result<SignInStart, String> {
    sign_in_card(app_handle, app, provider_id)?;
    let verifier = match app {
        SignInApp::Codex => random_b64url(64),
        SignInApp::Claude => random_b64url(32),
    };
    let state = random_b64url(32);
    let now = now();
    let expires_at = now + SIGN_IN_TTL_SECS;
    let session_id = uuid::Uuid::new_v4().to_string();

    let mut listener = listener_slot();
    let (redirect_uri, finish) = match app {
        SignInApp::Codex => match ensure_listener(&mut listener, app_handle) {
            Some(port) => (codex_redirect_uri(port), "callback"),
            None => (codex_redirect_uri(CODEX_CALLBACK_PORTS[0]), "code"),
        },
        SignInApp::Claude => (CLAUDE_REDIRECT_URI.to_string(), "code"),
    };
    let url = authorize_url(app, &pkce_challenge(&verifier), &state, &redirect_uri);
    sessions().begin(
        Pending {
            session_id: session_id.clone(),
            app,
            provider_id: provider_id.to_string(),
            verifier,
            state,
            redirect_uri,
            expires_at,
            finishing: false,
        },
        now,
    );
    drop(listener);

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(SIGN_IN_TTL_SECS as u64 + 1)).await;
        sessions().expire(self::now());
        stop_listener_if_idle();
    });
    log::info!(
        "[sign_in] started a {} sign-in of provider={provider_id} (finish by {finish})",
        app.name()
    );
    Ok(SignInStart {
        session_id,
        url,
        expires_at,
        finish,
    })
}

/// Finishes a sign-in with what the browser gave: the bare code, Claude's
/// `code#state`, or the whole address the browser was sent back to.
pub async fn finish(
    app_handle: &AppHandle,
    session_id: &str,
    input: &str,
) -> Result<SignedIn, String> {
    let pending = sessions().claim_session(session_id, now())?;
    let code = match parse_code(input, &pending.state) {
        Ok(code) => code,
        Err(e) => {
            sessions().release(session_id);
            return Err(e);
        }
    };
    complete(app_handle, &pending, code).await
}

pub fn status(session_id: &str) -> Result<SignInStatus, String> {
    sessions()
        .status(session_id, now())
        .ok_or_else(|| format!("no sign-in {session_id}"))
}

/// What a finish was given, reduced to the code. `Err` with the provider's
/// own message when the address carries an error, or when the input is from
/// another sign-in.
fn parse_code(input: &str, expected_state: &str) -> Result<String, String> {
    let input = input.trim();
    let wrong_state =
        || "this code belongs to another sign-in (its state does not match)".to_string();
    if input.starts_with("http://") || input.starts_with("https://") {
        let url = url::Url::parse(input).map_err(|e| format!("unreadable address: {e}"))?;
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        // Claude's callback page carries the state after `#` in the code.
        let (code, embedded_state) = match query.get("code") {
            Some(code) => match code.split_once('#') {
                Some((c, s)) => (Some(c.to_string()), Some(s.to_string())),
                None => (Some(code.clone()), None),
            },
            None => (None, None),
        };
        let state = query.get("state").cloned().or(embedded_state);
        if !state
            .as_deref()
            .is_some_and(|s| state_matches(s, expected_state))
        {
            return Err(wrong_state());
        }
        if let Some(error) = query.get("error") {
            let description = query.get("error_description").map(String::as_str);
            return Err(match description.filter(|d| !d.trim().is_empty()) {
                Some(d) => format!("the sign-in was refused: {d}"),
                None => format!("the sign-in was refused: {error}"),
            });
        }
        return code
            .filter(|c| !c.is_empty())
            .ok_or_else(|| "the address carries no code".to_string());
    }
    let code = match input.split_once('#') {
        Some((code, state)) => {
            if !state_matches(state, expected_state) {
                return Err(wrong_state());
            }
            code
        }
        None => input,
    };
    if code.is_empty() {
        return Err("no code given".to_string());
    }
    Ok(code.to_string())
}

/// Exchanges the code, checks and files the login, and records the outcome.
async fn complete(
    app_handle: &AppHandle,
    pending: &Pending,
    code: String,
) -> Result<SignedIn, String> {
    let result = exchange_and_file(app_handle, pending, &code).await;
    let outcome = match &result {
        Ok(signed) => Outcome::Done(signed.clone()),
        Err(message) => Outcome::Failed(message.clone()),
    };
    sessions().settle(&pending.session_id, &pending.provider_id, outcome, now());
    if pending.app == SignInApp::Codex {
        stop_listener_if_idle();
    }
    match &result {
        Ok(signed) => log::info!(
            "[sign_in] provider={} signed in as {}",
            signed.provider_id,
            signed.email
        ),
        Err(e) => log::warn!(
            "[sign_in] the sign-in of provider={} failed: {e}",
            pending.provider_id
        ),
    }
    result
}

async fn exchange_and_file(
    app_handle: &AppHandle,
    pending: &Pending,
    code: &str,
) -> Result<SignedIn, String> {
    let mut provider = sign_in_card(app_handle, pending.app, &pending.provider_id)?;
    let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
    let client = crate::proxy::http_client::get_for_provider(proxy_config)?;
    let db = app_handle.state::<AppState>().db.clone();

    match pending.app {
        SignInApp::Codex => {
            let reply = post_token(
                SignInApp::Codex,
                client
                    .post(codex_pool::TOKEN_ENDPOINT)
                    .form(&codex_exchange_form(code, pending)),
            )
            .await?;
            let login = codex_auth_from_reply(&reply, chrono::Utc::now(), &random_b64url(32))
                .ok_or("Codex's token reply carried no complete login")?;
            let new = codex_account::inspect(&login).unwrap_or_default();
            let account_id = new
                .account_key()
                .ok_or("the new login names no account")?
                .to_string();
            let email = new.email.clone().unwrap_or_default();
            let held = provider
                .settings_config
                .get("auth")
                .and_then(codex_account::inspect)
                .filter(|l| l.account_key().is_some());
            check_same_account(
                held.as_ref().map(|l| HeldAccount {
                    key: l.account_key().unwrap_or_default(),
                    email: l.email.as_deref(),
                }),
                &account_id,
                &email,
            )?;
            codex_pool::file_sign_in(&db, &mut provider, &login).map_err(|e| e.to_string())?;
            Ok(SignedIn {
                email,
                account_id,
                provider_id: provider.id,
            })
        }
        SignInApp::Claude => {
            let reply = post_token(
                SignInApp::Claude,
                client
                    .post(claude_pool::TOKEN_ENDPOINT)
                    .header("Accept", "application/json, text/plain, */*")
                    .header("User-Agent", claude_pool::REFRESH_USER_AGENT)
                    .json(&claude_exchange_body(code, pending)),
            )
            .await?;
            let access = reply
                .get("access_token")
                .and_then(Value::as_str)
                .ok_or("Claude's token reply carried no access token")?;
            let profile = match fetch_claude_profile(&client, access).await {
                Ok(profile) => Some(profile),
                Err(e) => {
                    log::warn!("[sign_in] could not read the Claude profile: {e}");
                    None
                }
            };
            let now_ms = chrono::Utc::now().timestamp_millis();
            let oauth_account = claude_oauth_account(&reply, profile.as_ref(), now_ms)
                .ok_or("the sign-in did not name its account")?;
            let account_id = oauth_account["accountUuid"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let email = oauth_account["emailAddress"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let held = provider
                .meta
                .as_ref()
                .and_then(|m| m.captured_claude_account.as_ref())
                .filter(|c| !c.account_uuid.is_empty());
            check_same_account(
                held.map(|c| HeldAccount {
                    key: &c.account_uuid,
                    email: Some(&c.email_address),
                }),
                &account_id,
                &email,
            )?;
            let credentials = claude_credentials_from_reply(&reply, profile.as_ref(), now_ms)
                .ok_or("Claude's token reply carried no complete login")?;
            crate::services::claude_account::file_sign_in(
                &db,
                &provider,
                credentials,
                oauth_account,
            )
            .map_err(|e| e.to_string())?;
            claude_pool::clear_refused(&provider.id);
            codex_pool::announce_sign_in("claude", &provider.id);
            Ok(SignedIn {
                email,
                account_id,
                provider_id: provider.id,
            })
        }
    }
}

/// Sends a code exchange and answers the reply, or the server's message.
async fn post_token(app: SignInApp, request: reqwest::RequestBuilder) -> Result<Value, String> {
    let response = request
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("could not reach {}'s token endpoint: {e}", app.name()))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{} refused the sign-in code ({status}): {}",
            app.name(),
            body.chars().take(300).collect::<String>()
        ));
    }
    serde_json::from_str(&body)
        .map_err(|e| format!("{}'s token reply is unreadable: {e}", app.name()))
}

fn codex_exchange_form<'a>(code: &'a str, pending: &'a Pending) -> Vec<(&'static str, &'a str)> {
    vec![
        ("grant_type", "authorization_code"),
        ("client_id", codex_pool::CLIENT_ID),
        ("code", code),
        ("redirect_uri", &pending.redirect_uri),
        ("code_verifier", &pending.verifier),
    ]
}

fn claude_exchange_body(code: &str, pending: &Pending) -> Value {
    json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": pending.redirect_uri,
        "client_id": claude_pool::CLIENT_ID,
        "code_verifier": pending.verifier,
        "state": pending.state,
    })
}

async fn fetch_claude_profile(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<Value, String> {
    let response = client
        .get(CLAUDE_PROFILE_URL)
        .header("User-Agent", CLAUDE_PROFILE_USER_AGENT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("profile answered {status}"));
    }
    response.json().await.map_err(|e| e.to_string())
}

// ── Shaping the login ───────────────────────────────────────────────────────

/// The `auth.json` `codex login` writes for a token reply: its tokens, the
/// account id from the id token's `chatgpt_account_id` claim, the OAuth
/// authority they were issued by, and `last_refresh`. Codex keeps
/// `OPENAI_API_KEY` empty for a ChatGPT login Switchy serves.
fn codex_auth_from_reply(
    reply: &Value,
    now: chrono::DateTime<chrono::Utc>,
    login_owner_id: &str,
) -> Option<Value> {
    let token = |key: &str| {
        reply
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let (id_token, access_token, refresh_token) = (
        token("id_token")?,
        token("access_token")?,
        token("refresh_token")?,
    );
    let account_id = codex_account::decode_jwt_payload(&id_token).and_then(|claims| {
        claims
            .get("https://api.openai.com/auth")?
            .get("chatgpt_account_id")?
            .as_str()
            .map(str::to_string)
    });
    Some(json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "oauth_provenance": { "issuer": CODEX_ISSUER, "client_id": codex_pool::CLIENT_ID },
            "login_owner_id": login_owner_id,
            "account_id": account_id,
        },
        "last_refresh": now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
    }))
}

/// Claude Code's plan name for an organization type.
fn subscription_type(organization_type: Option<&str>) -> Option<&'static str> {
    match organization_type? {
        "claude_max" => Some("max"),
        "claude_pro" => Some("pro"),
        "claude_enterprise" => Some("enterprise"),
        "claude_team" => Some("team"),
        _ => None,
    }
}

/// The `claudeAiOauth` block Claude Code stores after `/login`.
fn claude_credentials_from_reply(
    reply: &Value,
    profile: Option<&Value>,
    now_ms: i64,
) -> Option<Value> {
    let token = |key: &str| {
        reply
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let (access, refresh) = (token("access_token")?, token("refresh_token")?);
    let seconds = |key: &str| reply.get(key).and_then(Value::as_i64).filter(|s| *s > 0);
    let organization = profile.and_then(|p| p.get("organization"));
    let plan = subscription_type(
        organization
            .and_then(|o| o.get("organization_type"))
            .and_then(Value::as_str),
    );
    let scopes: Vec<&str> = reply
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(' ')
        .filter(|s| !s.is_empty())
        .collect();
    let mut oauth = json!({
        "accessToken": access,
        "refreshToken": refresh,
        "expiresAt": now_ms + seconds("expires_in").unwrap_or(3600) * 1000,
        "refreshTokenExpiresAt": seconds("refresh_token_expires_in")
            .map(|s| now_ms + s * 1000)
            .unwrap_or(now_ms + CLAUDE_REFRESH_TTL_MS),
        "scopes": scopes,
        "subscriptionType": plan,
    });
    if plan.is_some() {
        oauth["subscriptionTypeSource"] = json!("profile");
    }
    oauth["rateLimitTier"] = organization
        .and_then(|o| o.get("rate_limit_tier"))
        .cloned()
        .unwrap_or(Value::Null);
    Some(oauth)
}

/// The `oauthAccount` block Claude Code writes from the profile after
/// `/login`; from the token reply's account alone when the profile could not
/// be read.
fn claude_oauth_account(reply: &Value, profile: Option<&Value>, now_ms: i64) -> Option<Value> {
    let text = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(profile) = profile {
        let account = profile.get("account");
        let organization = profile.get("organization");
        if let (Some(uuid), Some(email), Some(org)) = (
            text(account.and_then(|a| a.get("uuid"))),
            text(account.and_then(|a| a.get("email"))),
            organization,
        ) {
            let org_field = |key: &str| org.get(key).cloned().filter(|v| !v.is_null());
            let mut out = serde_json::Map::new();
            out.insert("accountUuid".into(), json!(uuid));
            out.insert("emailAddress".into(), json!(email));
            if let Some(v) = text(org.get("uuid")) {
                out.insert("organizationUuid".into(), json!(v));
            }
            for (key, from) in [("displayName", "display_name"), ("fullName", "full_name")] {
                if let Some(v) = text(account.and_then(|a| a.get(from))) {
                    out.insert(key.into(), json!(v));
                }
            }
            out.insert(
                "hasExtraUsageEnabled".into(),
                json!(org
                    .get("has_extra_usage_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)),
            );
            if let Some(v) = org_field("billing_type") {
                out.insert("billingType".into(), v);
            }
            if let Some(v) = account.and_then(|a| a.get("created_at")).cloned() {
                out.insert("accountCreatedAt".into(), v);
            }
            if let Some(v) = org_field("subscription_created_at") {
                out.insert("subscriptionCreatedAt".into(), v);
            }
            out.insert(
                "ccOnboardingFlags".into(),
                org_field("cc_onboarding_flags").unwrap_or_else(|| json!({})),
            );
            for (key, from) in [
                ("claudeCodeTrialEndsAt", "claude_code_trial_ends_at"),
                (
                    "claudeCodeTrialDurationDays",
                    "claude_code_trial_duration_days",
                ),
                ("seatTier", "seat_tier"),
            ] {
                out.insert(key.into(), org_field(from).unwrap_or(Value::Null));
            }
            if let Some(v) = text(org.get("plan_display_name")) {
                out.insert("planDisplayName".into(), json!(v));
            }
            if let Some(v) = text(org.get("name")) {
                out.insert("organizationName".into(), json!(v));
            }
            out.insert("profileFetchedAt".into(), json!(now_ms));
            return Some(Value::Object(out));
        }
    }
    let account = reply.get("account");
    let uuid = text(account.and_then(|a| a.get("uuid")))?;
    let email = text(account.and_then(|a| a.get("email_address")))?;
    let mut out = json!({ "accountUuid": uuid, "emailAddress": email });
    let organization = reply.get("organization");
    if let Some(v) = text(organization.and_then(|o| o.get("uuid"))) {
        out["organizationUuid"] = json!(v);
    }
    if let Some(v) = text(organization.and_then(|o| o.get("name"))) {
        out["organizationName"] = json!(v);
    }
    Some(out)
}

/// The account a card already holds a login of.
struct HeldAccount<'a> {
    key: &'a str,
    email: Option<&'a str>,
}

/// A card holding a login takes a new one only of the same account; a card
/// with none takes any.
fn check_same_account(
    held: Option<HeldAccount<'_>>,
    new_key: &str,
    new_email: &str,
) -> Result<(), String> {
    let Some(held) = held else {
        return Ok(());
    };
    if held.key == new_key {
        return Ok(());
    }
    let held_email = held.email.filter(|e| !e.is_empty()).unwrap_or(held.key);
    let new_email = if new_email.is_empty() {
        new_key
    } else {
        new_email
    };
    Err(format!(
        "this card holds {held_email}, but the sign-in was for {new_email}; nothing was changed. Sign in as {held_email}, or use another card for {new_email}."
    ))
}

// ── Codex's callback ────────────────────────────────────────────────────────

struct Listener {
    port: u16,
    stop: tokio::sync::oneshot::Sender<()>,
}

static LISTENER: Lazy<Mutex<Option<Listener>>> = Lazy::new(|| Mutex::new(None));

/// Held while starting a sign-in and while deciding to stop listening, so a
/// sign-in started meanwhile is never left without its listener.
fn listener_slot() -> std::sync::MutexGuard<'static, Option<Listener>> {
    LISTENER.lock().unwrap_or_else(|e| e.into_inner())
}

/// The port Switchy answers Codex's callback on, listening first if it is
/// not yet. `None` when both of Codex's ports are taken (a `codex login` is
/// running): the sign-in is then finished with the address the browser lands on.
fn ensure_listener(slot: &mut Option<Listener>, app_handle: &AppHandle) -> Option<u16> {
    if let Some(listener) = slot.as_ref() {
        return Some(listener.port);
    }
    let (std_listener, port) = CODEX_CALLBACK_PORTS.iter().find_map(|port| {
        std::net::TcpListener::bind(("127.0.0.1", *port))
            .ok()
            .map(|l| (l, *port))
    })?;
    if let Err(e) = std_listener.set_nonblocking(true) {
        log::warn!("[sign_in] could not listen on {port}: {e}");
        return None;
    }
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(e) => {
                log::warn!("[sign_in] could not listen on {port}: {e}");
                return;
            }
        };
        let router = axum::Router::new()
            .route("/auth/callback", axum::routing::get(codex_callback))
            .with_state(app_handle);
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
        if let Err(e) = served {
            log::warn!("[sign_in] the Codex callback listener stopped: {e}");
        }
    });
    log::info!("[sign_in] answering Codex's sign-in callback on 127.0.0.1:{port}");
    *slot = Some(Listener { port, stop });
    Some(port)
}

fn stop_listener_if_idle() {
    let mut slot = listener_slot();
    if sessions().codex_pending() {
        return;
    }
    if let Some(listener) = slot.take() {
        let _ = listener.stop.send(());
        log::info!("[sign_in] stopped answering Codex's sign-in callback");
    }
}

async fn codex_callback(
    axum::extract::State(app_handle): axum::extract::State<AppHandle>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> (
    axum::http::StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    String,
) {
    use axum::http::{header, StatusCode};
    let page = |status, text: String| {
        (
            status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            text,
        )
    };
    let state = query.get("state").map(String::as_str).unwrap_or_default();
    let pending = match sessions().claim_state(state, now()) {
        Ok(pending) => pending,
        Err(e) => return page(StatusCode::BAD_REQUEST, e),
    };
    if let Some(error) = query.get("error") {
        let message = format!(
            "the sign-in was refused: {}",
            query
                .get("error_description")
                .filter(|d| !d.trim().is_empty())
                .unwrap_or(error)
        );
        sessions().settle(
            &pending.session_id,
            &pending.provider_id,
            Outcome::Failed(message.clone()),
            now(),
        );
        stop_listener_if_idle();
        return page(
            StatusCode::BAD_REQUEST,
            format!("Sign-in failed: {message}"),
        );
    }
    let Some(code) = query.get("code").filter(|c| !c.is_empty()).cloned() else {
        sessions().release(&pending.session_id);
        return page(
            StatusCode::BAD_REQUEST,
            "Sign-in failed: the address carries no code.".to_string(),
        );
    };
    match complete(&app_handle, &pending, code).await {
        Ok(signed) => page(
            StatusCode::OK,
            format!("Signed in as {}. You can close this tab.", signed.email),
        ),
        Err(e) => page(StatusCode::BAD_REQUEST, format!("Sign-in failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload: Value) -> String {
        format!(
            "{}.{}.{}",
            b64url(br#"{"alg":"RS256"}"#),
            b64url(&serde_json::to_vec(&payload).unwrap()),
            b64url(b"sig")
        )
    }

    fn pending(app: SignInApp, provider_id: &str, session_id: &str, expires_at: i64) -> Pending {
        Pending {
            session_id: session_id.into(),
            app,
            provider_id: provider_id.into(),
            verifier: "VER".into(),
            state: format!("state-{session_id}"),
            redirect_uri: codex_redirect_uri(1455),
            expires_at,
            finishing: false,
        }
    }

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier() {
        // RFC 7636 appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let verifier = random_b64url(64);
        assert_eq!(verifier.len(), 86, "64 bytes, as codex login draws");
        assert_eq!(
            random_b64url(32).len(),
            43,
            "32 bytes, as Claude Code draws"
        );
        assert_ne!(random_b64url(32), random_b64url(32));
    }

    fn query(url: &str) -> Vec<(String, String)> {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[test]
    fn the_codex_link_carries_what_codex_login_sends() {
        let url = authorize_url(SignInApp::Codex, "CH", "ST", &codex_redirect_uri(1455));
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        let expected: Vec<(String, String)> = [
            ("response_type", "code"),
            ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann"),
            ("redirect_uri", "http://127.0.0.1:1455/auth/callback"),
            ("code_challenge", "CH"),
            ("code_challenge_method", "S256"),
            ("state", "ST"),
            (
                "scope",
                "openid profile email offline_access api.connectors.read api.connectors.invoke",
            ),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(query(&url), expected);
    }

    #[test]
    fn the_claude_link_carries_what_claude_login_sends() {
        let url = authorize_url(SignInApp::Claude, "CH", "ST", CLAUDE_REDIRECT_URI);
        assert!(url.starts_with("https://claude.com/cai/oauth/authorize?"));
        let expected: Vec<(String, String)> = [
            ("code", "true"),
            ("client_id", "9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
            ("response_type", "code"),
            ("redirect_uri", "https://platform.claude.com/oauth/code/callback"),
            (
                "scope",
                "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload user:plugins",
            ),
            ("code_challenge", "CH"),
            ("code_challenge_method", "S256"),
            ("state", "ST"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(query(&url), expected);
    }

    #[test]
    fn a_finish_takes_a_bare_code_a_code_with_its_state_or_the_callback_address() {
        assert_eq!(parse_code("  abc  ", "ST").unwrap(), "abc");
        assert_eq!(parse_code("abc#ST", "ST").unwrap(), "abc");
        assert_eq!(
            parse_code(
                "http://127.0.0.1:1455/auth/callback?code=abc&scope=x&state=ST",
                "ST"
            )
            .unwrap(),
            "abc"
        );
        assert_eq!(
            parse_code(
                "https://platform.claude.com/oauth/code/callback?code=abc%23ST&state=ST",
                "ST"
            )
            .unwrap(),
            "abc"
        );
        assert_eq!(
            parse_code(
                "https://platform.claude.com/oauth/code/callback?code=abc%23ST",
                "ST"
            )
            .unwrap(),
            "abc",
            "the state inside the code is enough"
        );
    }

    #[test]
    fn a_finish_refuses_another_sign_ins_code_and_an_error() {
        assert!(parse_code("abc#OTHER", "ST")
            .unwrap_err()
            .contains("another sign-in"));
        assert!(parse_code(
            "http://127.0.0.1:1455/auth/callback?code=abc&state=OTHER",
            "ST"
        )
        .unwrap_err()
        .contains("another sign-in"));
        assert!(parse_code("http://127.0.0.1:1455/auth/callback?code=abc", "ST").is_err());
        assert!(parse_code(
            "http://127.0.0.1:1455/auth/callback?state=ST&error=access_denied&error_description=User%20said%20no",
            "ST"
        )
        .unwrap_err()
        .contains("User said no"));
        assert!(parse_code("", "ST").is_err());
        assert!(parse_code("#ST", "ST").is_err());
    }

    #[test]
    fn a_codex_reply_becomes_auth_json_as_codex_login_writes_it() {
        let id_token = jwt(json!({
            "email": "a@x.io",
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct-a", "chatgpt_plan_type": "pro" }
        }));
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-26T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let auth = codex_auth_from_reply(
            &json!({ "id_token": id_token, "access_token": "AAA", "refresh_token": "RRR", "expires_in": 864000 }),
            now,
            "OWNER",
        )
        .unwrap();
        assert_eq!(
            auth,
            json!({
                "auth_mode": "chatgpt",
                "OPENAI_API_KEY": null,
                "tokens": {
                    "id_token": id_token,
                    "access_token": "AAA",
                    "refresh_token": "RRR",
                    "oauth_provenance": { "issuer": "https://auth.openai.com", "client_id": "app_EMoamEEZ73f0CkXaXp7hrann" },
                    "login_owner_id": "OWNER",
                    "account_id": "acct-a",
                },
                "last_refresh": "2026-10-26T12:00:00.000000000Z",
            })
        );
        let login = codex_account::inspect(&auth).unwrap();
        assert!(login.alive);
        assert_eq!(login.account_key(), Some("acct-a"));
        assert_eq!(login.email.as_deref(), Some("a@x.io"));
        assert!(
            crate::proxy::codex_pool::is_chatgpt_live_auth(&auth),
            "served as a ChatGPT login"
        );
        assert!(codex_auth_from_reply(&json!({ "access_token": "AAA" }), now, "O").is_none());
    }

    fn claude_profile() -> Value {
        json!({
            "account": {
                "uuid": "u-1",
                "email": "a@x.io",
                "display_name": "A",
                "full_name": "A X",
                "created_at": "2025-01-01T00:00:00Z"
            },
            "organization": {
                "uuid": "o-1",
                "name": "A's Organization",
                "organization_type": "claude_max",
                "rate_limit_tier": "default_claude_max_20x",
                "billing_type": "stripe_subscription",
                "has_extra_usage_enabled": true,
                "subscription_created_at": "2025-02-01T00:00:00Z",
                "cc_onboarding_flags": {},
                "seat_tier": null
            }
        })
    }

    fn claude_reply() -> Value {
        json!({
            "access_token": "AAA",
            "refresh_token": "RRR",
            "expires_in": 28800,
            "scope": "user:inference user:profile user:sessions:claude_code",
            "account": { "uuid": "u-1", "email_address": "a@x.io" },
            "organization": { "uuid": "o-1", "name": "A's Organization" }
        })
    }

    #[test]
    fn a_claude_reply_becomes_the_credentials_claude_code_stores() {
        let oauth =
            claude_credentials_from_reply(&claude_reply(), Some(&claude_profile()), 1_000).unwrap();
        assert_eq!(
            oauth,
            json!({
                "accessToken": "AAA",
                "refreshToken": "RRR",
                "expiresAt": 1_000 + 28_800_000,
                "refreshTokenExpiresAt": 1_000 + CLAUDE_REFRESH_TTL_MS,
                "scopes": ["user:inference", "user:profile", "user:sessions:claude_code"],
                "subscriptionType": "max",
                "subscriptionTypeSource": "profile",
                "rateLimitTier": "default_claude_max_20x",
            })
        );
        let (alive, expires) = crate::services::credential_mirror::credential_health(
            &json!({ "claudeAiOauth": oauth }),
        );
        assert!(alive);
        assert_eq!(expires, 1_000 + 28_800_000);

        let unprofiled = claude_credentials_from_reply(&claude_reply(), None, 0).unwrap();
        assert_eq!(unprofiled["subscriptionType"], Value::Null);
        assert!(unprofiled.get("subscriptionTypeSource").is_none());
    }

    #[test]
    fn a_claude_profile_becomes_the_oauth_account_block() {
        let block = claude_oauth_account(&claude_reply(), Some(&claude_profile()), 5).unwrap();
        assert_eq!(block["accountUuid"], "u-1");
        assert_eq!(block["emailAddress"], "a@x.io");
        assert_eq!(block["organizationUuid"], "o-1");
        assert_eq!(block["organizationName"], "A's Organization");
        assert_eq!(block["displayName"], "A");
        assert_eq!(block["hasExtraUsageEnabled"], true);
        assert_eq!(block["billingType"], "stripe_subscription");
        assert_eq!(block["seatTier"], Value::Null);
        assert_eq!(block["profileFetchedAt"], 5);

        let fallback = claude_oauth_account(&claude_reply(), None, 5).unwrap();
        assert_eq!(
            fallback,
            json!({
                "accountUuid": "u-1",
                "emailAddress": "a@x.io",
                "organizationUuid": "o-1",
                "organizationName": "A's Organization",
            })
        );
        assert!(claude_oauth_account(&json!({}), None, 5).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn a_claude_sign_in_leaves_the_card_as_a_capture_does() {
        let home = tempfile::TempDir::new().expect("temp home");
        std::env::set_var(crate::paths::ENV_TEST_HOME, home.path());
        let db = crate::database::Database::memory().expect("db");
        let mut card = Provider::with_id(
            "signin-claude".into(),
            "C".into(),
            json!({ "env": {} }),
            None,
        );
        card.category = Some("official".into());
        db.save_provider("claude", &card).expect("save");
        let cred_path = crate::services::claude_account::paths::snapshot_credentials_path(&card.id);
        std::fs::create_dir_all(cred_path.parent().unwrap()).unwrap();
        std::fs::write(
            &cred_path,
            r#"{"claudeAiOauth":{"refreshToken":"old"},"mcpOAuth":{"k":1}}"#,
        )
        .unwrap();

        let oauth =
            claude_credentials_from_reply(&claude_reply(), Some(&claude_profile()), 1).unwrap();
        let block = claude_oauth_account(&claude_reply(), Some(&claude_profile()), 1).unwrap();
        let identity =
            crate::services::claude_account::file_sign_in(&db, &card, oauth, block.clone())
                .unwrap();

        use crate::services::claude_account::{paths, store};
        let creds = store::read_snapshot(&cred_path).unwrap();
        let oauth_account =
            store::read_snapshot(&paths::snapshot_oauth_account_path(&card.id)).unwrap();
        let account_state =
            store::read_snapshot(&paths::snapshot_account_state_path(&card.id)).unwrap();
        let stored = db.get_provider_by_id(&card.id, "claude").unwrap().unwrap();
        std::env::remove_var(crate::paths::ENV_TEST_HOME);

        assert_eq!(identity.account_uuid, "u-1");
        assert_eq!(creds["claudeAiOauth"]["refreshToken"], "RRR");
        assert_eq!(
            creds["mcpOAuth"],
            json!({ "k": 1 }),
            "the card's other keys stay"
        );
        assert_eq!(oauth_account, block);
        assert_eq!(account_state, json!({}));
        assert!(crate::proxy::claude_pool::is_oauth_provider(&stored));
        let captured = stored.meta.unwrap().captured_claude_account.unwrap();
        assert_eq!(captured.account_uuid, "u-1");
        assert_eq!(captured.email_address, "a@x.io");
    }

    #[test]
    fn a_card_holding_another_account_refuses_the_sign_in() {
        assert!(
            check_same_account(None, "acct-b", "b@x.io").is_ok(),
            "an empty card takes any"
        );
        let held = || {
            Some(HeldAccount {
                key: "acct-a",
                email: Some("a@x.io"),
            })
        };
        assert!(check_same_account(held(), "acct-a", "a@x.io").is_ok());
        let refused = check_same_account(held(), "acct-b", "b@x.io").unwrap_err();
        assert!(
            refused.contains("a@x.io") && refused.contains("b@x.io"),
            "{refused}"
        );
    }

    #[test]
    fn only_an_official_sign_in_card_can_be_signed_in() {
        let card = |category: &str, settings: Value| {
            let mut p = Provider::with_id("p".into(), "P".into(), settings, None);
            p.category = Some(category.into());
            p
        };
        assert!(is_sign_in_card(
            SignInApp::Codex,
            &card("official", json!({ "auth": {}, "config": "" }))
        ));
        assert!(!is_sign_in_card(
            SignInApp::Codex,
            &card(
                "official",
                json!({ "auth": { "OPENAI_API_KEY": "sk-x" }, "config": "" })
            )
        ));
        assert!(!is_sign_in_card(
            SignInApp::Codex,
            &card("third_party", json!({ "auth": {}, "config": "" }))
        ));
        assert!(!is_sign_in_card(
            SignInApp::Codex,
            &card(
                "official",
                json!({ "auth": {}, "config": "[model_providers.x]\nbase_url = \"https://x\"" })
            )
        ));
        assert!(is_sign_in_card(
            SignInApp::Claude,
            &card("official", json!({ "env": {} }))
        ));
        assert!(!is_sign_in_card(
            SignInApp::Claude,
            &card("custom", json!({ "env": {} }))
        ));
    }

    #[test]
    fn a_pending_sign_in_expires_and_a_new_start_replaces_it() {
        let mut sessions = Sessions::default();
        sessions.begin(
            pending(SignInApp::Codex, "card", "s1", 100 + SIGN_IN_TTL_SECS),
            100,
        );
        assert_eq!(sessions.status("s1", 101).unwrap().status, "pending");
        assert!(sessions.codex_pending());

        sessions.begin(
            pending(SignInApp::Codex, "card", "s2", 200 + SIGN_IN_TTL_SECS),
            200,
        );
        let replaced = sessions.status("s1", 201).unwrap();
        assert_eq!(replaced.status, "failed");
        assert!(sessions.claim_session("s1", 201).is_err());

        let at_expiry = 200 + SIGN_IN_TTL_SECS;
        assert_eq!(sessions.status("s2", at_expiry).unwrap().status, "expired");
        assert!(sessions
            .claim_session("s2", at_expiry)
            .unwrap_err()
            .contains("expired"));
        assert!(!sessions.codex_pending(), "nothing left to listen for");
    }

    #[test]
    fn a_sign_in_is_finished_once() {
        let mut sessions = Sessions::default();
        sessions.begin(pending(SignInApp::Codex, "card", "s1", 10_000), 0);
        let claimed = sessions.claim_state("state-s1", 1).unwrap();
        assert!(sessions
            .claim_session("s1", 1)
            .unwrap_err()
            .contains("already being finished"));
        assert_eq!(sessions.status("s1", 1).unwrap().status, "pending");
        sessions.settle(
            &claimed.session_id,
            &claimed.provider_id,
            Outcome::Done(SignedIn {
                email: "a@x.io".into(),
                account_id: "acct-a".into(),
                provider_id: "card".into(),
            }),
            2,
        );
        let done = sessions.status("s1", 3).unwrap();
        assert_eq!(
            (done.status, done.email.as_deref()),
            ("done", Some("a@x.io"))
        );
        assert!(sessions
            .claim_session("s1", 3)
            .unwrap_err()
            .contains("already finished"));

        sessions.begin(pending(SignInApp::Claude, "card", "s2", 10_000), 4);
        assert!(
            sessions.claim_state("state-s2", 5).is_err(),
            "the callback finishes only Codex"
        );
        sessions.claim_session("s2", 5).unwrap();
        sessions.release("s2");
        assert!(
            sessions.claim_session("s2", 6).is_ok(),
            "input that was not a code hands it back"
        );
    }
}
