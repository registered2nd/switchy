//! What the proxy needs to serve several subscription logins of one CLI as a
//! pool, whichever CLI it is: the settings, the quota each account last
//! reported, and the check that a login is never presented from the wrong
//! place. `codex_pool` and `claude_pool` hold what is specific to each login.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::error::ProxyError;

/// Quota-driven rotation settings. Stored in the settings table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountPoolConfig {
    /// Move to the next account in the failover queue before the current one
    /// is spent. Off by default.
    #[serde(default)]
    pub enabled: bool,
    /// Used-percent at which an account is passed over.
    #[serde(default = "default_threshold")]
    pub threshold_percent: u8,
    /// ISO country codes a ChatGPT login must never be presented from. The
    /// exit is checked before every such request and the request is refused
    /// when the exit is in one of these countries or cannot be established.
    /// Empty turns the check off.
    #[serde(default = "default_blocked_exit_countries")]
    pub blocked_exit_countries: Vec<String>,
    /// Open each pooled account's session window before it is needed by
    /// sending it one single-token request. Off by default: it spends quota
    /// with nobody present. See `keep_warm`.
    #[serde(default)]
    pub keep_warm_enabled: bool,
    /// How often each account is considered for a keep-warm request. A
    /// request goes out only when that account's session window is not
    /// already running.
    #[serde(default = "default_keep_warm_interval")]
    pub keep_warm_interval_minutes: u32,
}

fn default_threshold() -> u8 {
    98
}

fn default_blocked_exit_countries() -> Vec<String> {
    vec!["CN".to_string()]
}

fn default_keep_warm_interval() -> u32 {
    60
}

impl Default for AccountPoolConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_percent: default_threshold(),
            blocked_exit_countries: default_blocked_exit_countries(),
            keep_warm_enabled: false,
            keep_warm_interval_minutes: default_keep_warm_interval(),
        }
    }
}

// ── Exit check ──────────────────────────────────────────────────────────────
//
// A subscription login presented from the wrong place is refused with a 403
// that the CLI reads as a dead session, and the account has been seen there by then.
// So the check comes before the request: the proxy asks the edge in front of
// chatgpt.com where it sees this machine, over the same route the request will
// take, and sends nothing when the answer is a blocked country — or when there
// is no answer, which is what a fallen tunnel looks like from inside China.

const EXIT_OK_TTL_SECS: u64 = 30;
/// A refusal is remembered this long so the other accounts in the queue are
/// refused at once instead of each waiting out the hold.
const EXIT_REFUSAL_TTL_SECS: u64 = 10;
/// A tunnel that drops usually comes back within seconds; a held request looks
/// like a slow response, a refused one ends the turn.
const EXIT_HOLD_SECS: u64 = 30;
const EXIT_RECHECK_SECS: u64 = 3;

#[derive(Clone)]
struct ExitVerdict {
    allowed: bool,
    detail: String,
    checked_at: std::time::Instant,
}

static EXIT_VERDICTS: Lazy<Mutex<HashMap<String, ExitVerdict>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Pulls `ip` and `loc` out of a `/cdn-cgi/trace` body.
fn parse_exit_trace(body: &str) -> (Option<String>, Option<String>) {
    let field = |name: &str| {
        body.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    (field("ip"), field("loc"))
}

fn judge_exit(trace: Result<String, String>, blocked: &[String]) -> (bool, String) {
    match trace {
        Err(e) => (false, format!("the exit could not be established ({e})")),
        Ok(body) => match parse_exit_trace(&body) {
            (ip, Some(loc)) => {
                let ip = ip.unwrap_or_else(|| "unknown address".to_string());
                if blocked.iter().any(|c| c.eq_ignore_ascii_case(&loc)) {
                    (false, format!("traffic is leaving from {loc} ({ip})"))
                } else {
                    (true, format!("{loc} ({ip})"))
                }
            }
            _ => (false, "the exit check returned no country".to_string()),
        },
    }
}

async fn fetch_exit_trace(trace_url: &str, route: Option<&str>) -> Result<String, String> {
    let timeout = std::time::Duration::from_secs(10);
    if route.is_some_and(|r| r.starts_with("socks5")) {
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(route.unwrap_or_default()).map_err(|e| e.to_string())?)
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(trace_url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        return response.text().await.map_err(|e| e.to_string());
    }

    // Same client the model request itself goes out through.
    let uri: http::Uri = trace_url.parse().map_err(|e| format!("{e}"))?;
    let host = uri.host().unwrap_or_default().to_string();
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::HOST,
        http::HeaderValue::from_str(&host).map_err(|e| e.to_string())?,
    );
    headers.insert(http::header::ACCEPT, http::HeaderValue::from_static("*/*"));
    let response = crate::proxy::hyper_client::send_request(
        uri,
        http::Method::GET,
        headers,
        http::Extensions::new(),
        Vec::new(),
        timeout,
        route,
    )
    .await
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("exit check answered {}", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Refuses unless this machine's traffic is leaving from an allowed place over
/// the route `provider`'s requests take. `trace_url` is the `/cdn-cgi/trace` of
/// the host the login is about to be presented to.
pub async fn ensure_exit_allowed(
    db: &Database,
    provider: &Provider,
    trace_url: &str,
) -> Result<(), ProxyError> {
    let blocked = db
        .get_account_pool_config()
        .unwrap_or_default()
        .blocked_exit_countries;
    if blocked.is_empty() {
        return Ok(());
    }

    let route = provider
        .meta
        .as_ref()
        .and_then(|m| m.proxy_config.as_ref())
        .filter(|c| c.enabled)
        .and_then(crate::proxy::http_client::build_proxy_url_from_config)
        .or_else(crate::proxy::http_client::get_current_proxy_url);
    let route_key = format!("{}|{trace_url}", route.as_deref().unwrap_or("direct"));

    let cached = EXIT_VERDICTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&route_key)
        .cloned();
    if let Some(verdict) = cached {
        let ttl = if verdict.allowed {
            EXIT_OK_TTL_SECS
        } else {
            EXIT_REFUSAL_TTL_SECS
        };
        if verdict.checked_at.elapsed().as_secs() < ttl {
            return exit_result(&verdict);
        }
    }

    let started = std::time::Instant::now();
    let verdict = loop {
        let (allowed, detail) = judge_exit(
            fetch_exit_trace(trace_url, route.as_deref()).await,
            &blocked,
        );
        if allowed || started.elapsed().as_secs() >= EXIT_HOLD_SECS {
            break ExitVerdict {
                allowed,
                detail,
                checked_at: std::time::Instant::now(),
            };
        }
        log::warn!("[account_pool] holding request: {detail}");
        tokio::time::sleep(std::time::Duration::from_secs(EXIT_RECHECK_SECS)).await;
    };

    EXIT_VERDICTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(route_key, verdict.clone());
    exit_result(&verdict)
}

fn exit_result(verdict: &ExitVerdict) -> Result<(), ProxyError> {
    if verdict.allowed {
        return Ok(());
    }
    log::error!("[account_pool] request refused: {}", verdict.detail);
    Err(ProxyError::ForwardFailed(format!(
        "Switchy did not send this request: {}. Subscription account traffic only leaves through the proxy chain.",
        verdict.detail
    )))
}

// ── Quota ───────────────────────────────────────────────────────────────────

/// One rate-limit window as OpenAI reported it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// Model family the window is scoped to; `None` for the account-wide one.
    pub limit_name: Option<String>,
    pub window_minutes: i64,
    pub used_percent: f64,
    /// Unix seconds.
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuota {
    /// The account-wide windows from the latest response, and every
    /// model-scoped window last reported, by name.
    pub windows: Vec<QuotaWindow>,
    /// Unix seconds of the response these came from.
    pub observed_at: i64,
    /// Set when upstream refused the account outright; unix seconds.
    pub limited_until: Option<i64>,
    /// Model-scoped windows upstream refused, by window name; unix seconds.
    pub scoped_limited_until: HashMap<String, i64>,
}

static QUOTAS: Lazy<RwLock<HashMap<String, AccountQuota>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// For each model, the model-scoped windows its responses carry. Anthropic
/// reports only the windows that apply to the model that answered (a Fable
/// response carries `7d_oi`, an Opus one does not), so this is learned from
/// responses rather than known in advance.
static MODEL_SCOPES: Lazy<RwLock<HashMap<String, Vec<String>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Stores the windows a response reported for the account that served it.
/// Account-wide windows replace the previous ones; a model-scoped window
/// replaces only its own earlier reading, since a response for another model
/// does not carry it. `model` is the model the response answered, when known.
pub fn record_windows(provider_id: &str, model: Option<&str>, windows: Vec<QuotaWindow>) {
    if let Some(model) = model {
        let scoped = windows
            .iter()
            .filter_map(|w| w.limit_name.clone())
            .collect();
        MODEL_SCOPES
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(model.to_string(), scoped);
    }
    let mut quotas = QUOTAS.write().unwrap_or_else(|e| e.into_inner());
    let entry = quotas.entry(provider_id.to_string()).or_default();
    merge_windows(entry, windows);
    entry.observed_at = chrono::Utc::now().timestamp();
    entry.limited_until = None;
}

fn merge_windows(entry: &mut AccountQuota, windows: Vec<QuotaWindow>) {
    entry.windows.retain(|old| {
        old.limit_name.is_some()
            && !windows.iter().any(|new| {
                new.limit_name == old.limit_name && new.window_minutes == old.window_minutes
            })
    });
    for name in windows.iter().filter_map(|w| w.limit_name.as_ref()) {
        entry.scoped_limited_until.remove(name);
    }
    entry.windows.extend(windows);
    entry.windows.sort_by(|a, b| {
        (a.limit_name.is_some(), &a.limit_name, a.window_minutes).cmp(&(
            b.limit_name.is_some(),
            &b.limit_name,
            b.window_minutes,
        ))
    });
}

/// Records an outright usage-limit refusal, so the account is passed over
/// until `until` (unix seconds).
pub fn record_limited_until(provider_id: &str, until: i64) {
    let mut quotas = QUOTAS.write().unwrap_or_else(|e| e.into_inner());
    quotas
        .entry(provider_id.to_string())
        .or_default()
        .limited_until = Some(until);
    log::info!(
        "[account_pool] provider={provider_id} hit its usage limit; passing it over until {until}"
    );
}

/// Records a refusal on one model-scoped window, so the account is passed
/// over for the models that window applies to, and only those, until
/// `until` (unix seconds).
pub fn record_scoped_limited_until(provider_id: &str, window: &str, until: i64) {
    let mut quotas = QUOTAS.write().unwrap_or_else(|e| e.into_inner());
    quotas
        .entry(provider_id.to_string())
        .or_default()
        .scoped_limited_until
        .insert(window.to_string(), until);
    log::info!(
        "[account_pool] provider={provider_id} hit its {window} limit; passing it over for that model until {until}"
    );
}

/// Whether the account should be passed over for a request for `model`:
/// refused outright and not yet reset, an account-wide window at or past the
/// threshold, or a window scoped to that model at or past it. Windows scoped
/// to other models do not count. With no model, only account-wide ones do.
pub fn is_spent(provider_id: &str, model: Option<&str>, threshold_percent: u8, now: i64) -> bool {
    let quotas = QUOTAS.read().unwrap_or_else(|e| e.into_inner());
    let Some(quota) = quotas.get(provider_id) else {
        return false;
    };
    let scopes = MODEL_SCOPES.read().unwrap_or_else(|e| e.into_inner());
    quota_is_spent(
        quota,
        &applicable_scopes(&scopes, model),
        threshold_percent,
        now,
    )
}

/// The model-scoped windows that apply to `model`: the ones its responses
/// carry, and one named for the model itself, which is how Codex names them.
fn applicable_scopes(scopes: &HashMap<String, Vec<String>>, model: Option<&str>) -> Vec<String> {
    let Some(model) = model else {
        return Vec::new();
    };
    let mut names = scopes.get(model).cloned().unwrap_or_default();
    names.push(model.to_ascii_lowercase());
    names
}

fn window_applies(name: &str, scopes: &[String]) -> bool {
    let normalized = name.to_ascii_lowercase().replace(' ', "-");
    scopes
        .iter()
        .any(|s| s == name || s.to_ascii_lowercase() == normalized)
}

/// When the account's session window resets: the shortest account-wide
/// window, the one a single request opens. `None` when no response has
/// reported one, which is also what an account nobody has used looks like.
pub fn session_window_reset(provider_id: &str) -> Option<i64> {
    let quotas = QUOTAS.read().unwrap_or_else(|e| e.into_inner());
    session_reset_of(quotas.get(provider_id)?)
}

fn session_reset_of(quota: &AccountQuota) -> Option<i64> {
    quota
        .windows
        .iter()
        .filter(|w| w.limit_name.is_none())
        .min_by_key(|w| w.window_minutes)?
        .reset_at
}

fn quota_is_spent(
    quota: &AccountQuota,
    scopes: &[String],
    threshold_percent: u8,
    now: i64,
) -> bool {
    if quota.limited_until.is_some_and(|until| until > now) {
        return true;
    }
    if quota
        .scoped_limited_until
        .iter()
        .any(|(name, until)| *until > now && window_applies(name, scopes))
    {
        return true;
    }
    quota.windows.iter().any(|w| {
        w.limit_name
            .as_deref()
            .is_none_or(|name| window_applies(name, scopes))
            && w.used_percent >= f64::from(threshold_percent)
            // A window whose reset has passed is a stale reading of a fresh one.
            && w.reset_at.is_none_or(|reset| reset > now)
    })
}

/// Puts accounts that should be passed over for `model` at the back, keeping
/// queue order otherwise. They stay in the list: when every account is spent
/// the request still goes out and upstream's own answer reaches the client.
pub fn order_by_quota(
    providers: Vec<Provider>,
    model: Option<&str>,
    threshold_percent: u8,
) -> Vec<Provider> {
    let now = chrono::Utc::now().timestamp();
    let (fresh, spent): (Vec<_>, Vec<_>) = providers
        .into_iter()
        .partition(|p| !is_spent(&p.id, model, threshold_percent, now));
    fresh.into_iter().chain(spent).collect()
}

/// Quota last seen for each account, for the UI.
pub fn quota_snapshot() -> HashMap<String, AccountQuota> {
    QUOTAS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_in_a_blocked_country_is_refused() {
        let blocked = vec!["CN".to_string()];
        let (allowed, detail) = judge_exit(Ok("fl=1\nip=1.2.3.4\nloc=CN\n".into()), &blocked);
        assert!(!allowed);
        assert!(detail.contains("CN") && detail.contains("1.2.3.4"));
    }

    #[test]
    fn exit_elsewhere_is_allowed() {
        let blocked = vec!["cn".to_string()];
        let (allowed, _) = judge_exit(Ok("ip=12.17.210.193\nloc=US\nwarp=off\n".into()), &blocked);
        assert!(allowed);
    }

    #[test]
    fn unverifiable_exit_is_refused() {
        let blocked = vec!["CN".to_string()];
        assert!(!judge_exit(Err("connection reset".into()), &blocked).0);
        assert!(!judge_exit(Ok("ip=1.2.3.4\n".into()), &blocked).0);
        assert!(!judge_exit(Ok(String::new()), &blocked).0);
    }

    #[test]
    fn pool_config_defaults_block_china_and_tolerate_old_saved_values() {
        let saved: AccountPoolConfig =
            serde_json::from_str(r#"{"enabled":true,"thresholdPercent":95}"#).unwrap();
        assert_eq!(saved.blocked_exit_countries, vec!["CN".to_string()]);
        assert_eq!(
            AccountPoolConfig::default().blocked_exit_countries,
            vec!["CN".to_string()]
        );
    }

    #[test]
    fn keep_warm_is_off_in_defaults_and_in_settings_saved_before_it_existed() {
        let saved: AccountPoolConfig =
            serde_json::from_str(r#"{"enabled":true,"thresholdPercent":95}"#).unwrap();
        assert!(!saved.keep_warm_enabled);
        assert_eq!(saved.keep_warm_interval_minutes, 60);
        assert!(!AccountPoolConfig::default().keep_warm_enabled);
    }

    fn quota(used: f64, reset_at: Option<i64>, name: Option<&str>) -> AccountQuota {
        AccountQuota {
            windows: vec![QuotaWindow {
                limit_name: name.map(str::to_string),
                window_minutes: 10080,
                used_percent: used,
                reset_at,
            }],
            observed_at: 1000,
            limited_until: None,
            scoped_limited_until: HashMap::new(),
        }
    }

    fn fable() -> Vec<String> {
        vec!["7d_oi".to_string(), "claude-fable-5-1".to_string()]
    }

    #[test]
    fn account_window_past_threshold_is_spent_until_reset() {
        assert!(quota_is_spent(
            &quota(99.0, Some(2000), None),
            &[],
            98,
            1500
        ));
        assert!(!quota_is_spent(
            &quota(99.0, Some(2000), None),
            &[],
            98,
            2500
        ));
        assert!(!quota_is_spent(
            &quota(50.0, Some(2000), None),
            &[],
            98,
            1500
        ));
    }

    #[test]
    fn model_scoped_window_spends_the_account_only_for_its_model() {
        let q = quota(99.0, Some(2000), Some("7d_oi"));
        assert!(quota_is_spent(&q, &fable(), 98, 1500));
        assert!(!quota_is_spent(
            &q,
            &["claude-opus-5-5".to_string()],
            98,
            1500
        ));
        assert!(!quota_is_spent(&q, &[], 98, 1500));
    }

    #[test]
    fn a_codex_window_named_for_the_model_applies_to_it() {
        let q = quota(99.0, Some(2000), Some("GPT-5.3-Codex-Spark"));
        let spark = applicable_scopes(&HashMap::new(), Some("gpt-5.3-codex-spark"));
        let other = applicable_scopes(&HashMap::new(), Some("gpt-5.5"));
        assert!(quota_is_spent(&q, &spark, 98, 1500));
        assert!(!quota_is_spent(&q, &other, 98, 1500));
    }

    #[test]
    fn a_scoped_refusal_passes_the_account_over_only_for_its_model() {
        let mut q = quota(10.0, None, None);
        q.scoped_limited_until.insert("7d_oi".to_string(), 2000);
        assert!(quota_is_spent(&q, &fable(), 98, 1500));
        assert!(!quota_is_spent(&q, &fable(), 98, 2500));
        assert!(!quota_is_spent(&q, &[], 98, 1500));
    }

    #[test]
    fn a_response_for_another_model_keeps_the_scoped_reading() {
        let mut q = quota(90.0, Some(2000), Some("7d_oi"));
        merge_windows(
            &mut q,
            vec![QuotaWindow {
                limit_name: None,
                window_minutes: 10080,
                used_percent: 50.0,
                reset_at: Some(2000),
            }],
        );
        assert_eq!(q.windows.len(), 2);
        assert_eq!(q.windows[0].used_percent, 50.0);
        assert_eq!(q.windows[1].limit_name.as_deref(), Some("7d_oi"));
        assert_eq!(q.windows[1].used_percent, 90.0);
    }

    #[test]
    fn the_session_window_is_the_shortest_account_wide_one() {
        let mut q = quota(10.0, Some(2000), None);
        q.windows.push(QuotaWindow {
            limit_name: None,
            window_minutes: 300,
            used_percent: 5.0,
            reset_at: Some(1200),
        });
        q.windows.push(QuotaWindow {
            limit_name: Some("7d_oi".to_string()),
            window_minutes: 60,
            used_percent: 5.0,
            reset_at: Some(1100),
        });
        assert_eq!(session_reset_of(&q), Some(1200));
    }

    #[test]
    fn an_account_no_response_has_reported_on_has_no_session_window() {
        assert_eq!(session_reset_of(&AccountQuota::default()), None);
        let model_only = quota(10.0, Some(2000), Some("Spark"));
        assert_eq!(session_reset_of(&model_only), None);
    }

    #[test]
    fn refusal_spends_the_account_until_its_reset() {
        let mut q = quota(10.0, None, None);
        q.limited_until = Some(2000);
        assert!(quota_is_spent(&q, &[], 98, 1500));
        assert!(!quota_is_spent(&q, &[], 98, 2500));
    }
}
