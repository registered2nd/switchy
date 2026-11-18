//! What the proxy needs to serve several subscription logins of one CLI as a
//! pool, whichever CLI it is: the settings, the quota each account last
//! reported, and advisory observations of their exit regions. `codex_pool`
//! and `claude_pool` hold what is specific to each login.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::database::{Database, SwitchReason};
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
    /// ISO country codes to warn about in background exit observations.
    /// Empty turns the observations off. This field retains its saved name.
    #[serde(default = "default_blocked_exit_countries")]
    pub blocked_exit_countries: Vec<String>,
    /// Require an enforceable proxy or local TUN route for accounts without
    /// their own proxy. Router-proxied networks leave this off.
    #[serde(default)]
    pub require_proxy: bool,
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
    /// Sign Codex's shared background server in to the account the proxy
    /// serves, so `/status` in an open Codex window follows a switch. On by
    /// default; experimental. See `codex_engine`.
    #[serde(default = "default_codex_shared_session")]
    pub codex_shared_session: bool,
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

fn default_codex_shared_session() -> bool {
    true
}

impl Default for AccountPoolConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_percent: default_threshold(),
            blocked_exit_countries: default_blocked_exit_countries(),
            require_proxy: false,
            keep_warm_enabled: false,
            keep_warm_interval_minutes: default_keep_warm_interval(),
            codex_shared_session: default_codex_shared_session(),
        }
    }
}

// ── Exit check ──────────────────────────────────────────────────────────────
//
// Region observations never hold or refuse requests. Route enforcement belongs
// to the outbound client.
const EXIT_OBSERVATION_TTL_SECS: u64 = 300;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRouteStatus {
    pub route: String,
    pub target: String,
    pub country: Option<String>,
    pub ip: Option<String>,
    pub warning: Option<String>,
    pub checked_at: i64,
}

struct ExitObservation {
    status: ProxyRouteStatus,
    started_at: std::time::Instant,
    running: bool,
}

static EXIT_OBSERVATIONS: Lazy<Mutex<HashMap<String, ExitObservation>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn parse_exit_trace(body: &str) -> (Option<String>, Option<String>) {
    let field = |name: &str| {
        body.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    (field("ip"), field("loc"))
}

fn exit_status(
    route: String,
    target: String,
    trace: Result<String, String>,
    watched_countries: &[String],
    previous: Option<&ProxyRouteStatus>,
) -> ProxyRouteStatus {
    let parsed = trace.and_then(|body| {
        let (ip, country) = parse_exit_trace(&body);
        country
            .map(|country| (ip, country))
            .ok_or_else(|| "the exit check returned no country".to_string())
    });
    let (ip, country, warning) = match parsed {
        Ok((ip, country)) => {
            let warning = if watched_countries
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&country))
            {
                Some(format!(
                    "Exit region {country} needs attention. Country checks do not stop requests."
                ))
            } else if let Some(old) = previous
                .and_then(|s| s.country.as_deref())
                .filter(|old| !old.eq_ignore_ascii_case(&country))
            {
                Some(format!("Exit country changed from {old} to {country}. Country checks do not stop requests."))
            } else {
                None
            };
            (ip, Some(country), warning)
        }
        Err(error) => (
            previous.and_then(|s| s.ip.clone()),
            previous.and_then(|s| s.country.clone()),
            Some(format!(
                "Could not verify the exit region: {error}. Country checks do not stop requests."
            )),
        ),
    };
    ProxyRouteStatus {
        route,
        target,
        country,
        ip,
        warning,
        checked_at: chrono::Utc::now().timestamp(),
    }
}

async fn fetch_exit_trace(
    trace_url: &str,
    proxy_config: Option<&crate::provider::ProviderProxyConfig>,
) -> Result<String, String> {
    let response = super::http_client::get_for_provider(proxy_config)?
        .get(trace_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("exit check answered {}", response.status()));
    }
    response.text().await.map_err(|e| e.to_string())
}

/// Schedule a background region observation. The legacy name is kept for
/// callers; country lookups cannot prevent them from sending a request.
pub async fn ensure_exit_allowed(
    db: &Database,
    provider: &Provider,
    trace_url: &str,
) -> Result<(), ProxyError> {
    let watched = db
        .get_account_pool_config()
        .unwrap_or_default()
        .blocked_exit_countries;
    if watched.is_empty() {
        return Ok(());
    }
    let proxy_config = provider
        .meta
        .as_ref()
        .and_then(|m| m.proxy_config.as_ref())
        .cloned();
    let trace_url = trace_url.to_string();
    // Route discovery and network I/O happen after the request is released.
    tauri::async_runtime::spawn(async move {
        let route = super::http_client::effective_proxy_url(proxy_config.as_ref());
        let route_label = route
            .as_ref()
            .ok()
            .and_then(|r| r.as_deref())
            .map(super::http_client::mask_url)
            .unwrap_or_else(|| "Network route".to_string());
        let route_key = format!(
            "{}|{trace_url}",
            super::http_client::route_cache_key(proxy_config.as_ref())
                .unwrap_or_else(|_| route_label.clone())
        );
        let target = url::Url::parse(&trace_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let previous = {
            let mut observations = EXIT_OBSERVATIONS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(old) = observations.get(&route_key) {
                if old.running || old.started_at.elapsed().as_secs() < EXIT_OBSERVATION_TTL_SECS {
                    return;
                }
            }
            let previous = observations.get(&route_key).map(|old| old.status.clone());
            let status = previous.clone().unwrap_or(ProxyRouteStatus {
                route: route_label.clone(),
                target: target.clone(),
                country: None,
                ip: None,
                warning: None,
                checked_at: 0,
            });
            observations.insert(
                route_key.clone(),
                ExitObservation {
                    status,
                    started_at: std::time::Instant::now(),
                    running: true,
                },
            );
            previous
        };
        let result = match route {
            Ok(_) => fetch_exit_trace(&trace_url, proxy_config.as_ref()).await,
            Err(error) => Err(error),
        };
        let status = exit_status(route_label, target, result, &watched, previous.as_ref());
        if let Some(warning) = &status.warning {
            log::warn!(
                "[exit_region] {} via {}: {warning}",
                status.target,
                status.route
            );
        } else {
            log::info!(
                "[exit_region] {} via {}: {} ({})",
                status.target,
                status.route,
                status.country.as_deref().unwrap_or("unknown"),
                status.ip.as_deref().unwrap_or("unknown")
            );
        }
        let mut observations = EXIT_OBSERVATIONS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = observations.get_mut(&route_key) {
            old.status = status;
            old.running = false;
        }
        observations.retain(|_, old| old.running || old.started_at.elapsed().as_secs() < 3600);
    });
    Ok(())
}

pub fn proxy_route_status() -> Vec<ProxyRouteStatus> {
    let mut statuses: Vec<_> = EXIT_OBSERVATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|o| o.status.checked_at > 0 && o.started_at.elapsed().as_secs() < 3600)
        .map(|o| o.status.clone())
        .collect();
    statuses.sort_by(|a, b| b.checked_at.cmp(&a.checked_at));
    let mut seen = std::collections::HashSet::new();
    statuses.retain(|s| seen.insert((s.route.clone(), s.target.clone())));
    statuses
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
    /// When a usage reset was last taken, by window name and length; unix
    /// seconds.
    #[serde(skip)]
    usage_reset_at: HashMap<(Option<String>, i64), i64>,
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
pub fn record_windows(provider_id: &str, model: Option<&str>, mut windows: Vec<QuotaWindow>) {
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
    let now = chrono::Utc::now().timestamp();
    let mut quotas = QUOTAS.write().unwrap_or_else(|e| e.into_inner());
    let entry = quotas.entry(provider_id.to_string()).or_default();
    ignore_late_answers(provider_id, entry, &mut windows, now);
    let changed = if QUOTA_HISTORY.get().is_some() {
        changed_readings(&entry.windows, &windows)
    } else {
        Vec::new()
    };
    merge_windows(entry, windows);
    entry.observed_at = now;
    entry.limited_until = None;
    let observed_at = entry.observed_at;
    drop(quotas);
    save_readings(provider_id, &changed, observed_at);
}

/// Reset times this close count as the same: one window's reset moves by
/// seconds from one answer to the next, and two accounts' weeks can end
/// within seconds of each other.
const SAME_RESET_SECS: i64 = 3600;

/// Whether two readings are of the same window: same name and length, and
/// resets within `SAME_RESET_SECS` of each other.
fn same_window(a: &QuotaWindow, b: &QuotaWindow) -> bool {
    a.limit_name == b.limit_name
        && a.window_minutes == b.window_minutes
        && match (a.reset_at, b.reset_at) {
            (Some(x), Some(y)) => (x - y).abs() <= SAME_RESET_SECS,
            (x, y) => x == y,
        }
}

/// A reading at most this many points below the stored one of the same window
/// is an earlier answer arriving late; one further below is a usage reset,
/// which leaves the window's end where it was. In the readings kept to
/// 2026-11-14, 901 late answers read one or two points low; the resets read
/// 11 to 99 points lower (99 to 0 on 2026-11-14; 99 to 76 after a Fable reset
/// on 2026-11-18).
const LATE_ANSWER_MAX_DROP: f64 = 5.0;

/// For this long after a reset is taken, a reading more than
/// `LATE_ANSWER_MAX_DROP` above the stored one is an answer from before the
/// reset arriving late. Late answers have come up to two minutes behind a
/// newer one, and usage that is being read climbs a point or two at a time.
const LATE_ANSWER_SECS: i64 = 600;

/// Answers to requests running side by side arrive out of order, so a reading
/// a few points below the stored one of the same window is an older answer
/// and the stored one stands. Without this an account at its switch-away
/// limit reads 99, 98, 99 within seconds and is passed over and taken back in
/// turn. A reading further below is a reset the provider granted and is taken
/// at once; for `LATE_ANSWER_SECS` after it, a reading as far above the
/// stored one is an answer from before the reset, and the stored one stands.
fn ignore_late_answers(
    provider_id: &str,
    stored: &mut AccountQuota,
    windows: &mut [QuotaWindow],
    now: i64,
) {
    for new in windows.iter_mut().filter(|w| w.reset_at.is_some()) {
        let Some(old) = stored
            .windows
            .iter()
            .find(|old| same_window(old, new))
            .map(|old| old.used_percent)
        else {
            continue;
        };
        let key = (new.limit_name.clone(), new.window_minutes);
        if new.used_percent < old - LATE_ANSWER_MAX_DROP {
            log::info!(
                "[account_pool] provider={provider_id}: {} {}-minute window fell from {old:.0}% to {:.0}% before its reset; taking it as a usage reset",
                new.limit_name.as_deref().unwrap_or("account-wide"),
                new.window_minutes,
                new.used_percent
            );
            stored.usage_reset_at.insert(key, now);
        } else if new.used_percent < old
            || (new.used_percent > old + LATE_ANSWER_MAX_DROP
                && stored
                    .usage_reset_at
                    .get(&key)
                    .is_some_and(|at| now - at < LATE_ANSWER_SECS))
        {
            new.used_percent = old;
        }
    }
}

/// Where readings are kept; set once the database is open.
static QUOTA_HISTORY: std::sync::OnceLock<std::sync::Arc<crate::database::Database>> =
    std::sync::OnceLock::new();

/// Starts keeping each account's quota readings in the database.
pub fn keep_quota_history(db: std::sync::Arc<crate::database::Database>) {
    let _ = QUOTA_HISTORY.set(db);
}

/// The windows worth a history row: a whole-point change in the share used,
/// or a new window (its reset moved by more than an hour).
fn changed_readings(previous: &[QuotaWindow], windows: &[QuotaWindow]) -> Vec<QuotaWindow> {
    windows
        .iter()
        .filter(|new| {
            !previous.iter().any(|old| {
                same_window(old, new) && old.used_percent.floor() == new.used_percent.floor()
            })
        })
        .cloned()
        .collect()
}

fn save_readings(provider_id: &str, windows: &[QuotaWindow], observed_at: i64) {
    let Some(db) = QUOTA_HISTORY.get() else {
        return;
    };
    for w in windows {
        if let Err(e) = db.record_quota_reading(
            provider_id,
            w.limit_name.as_deref(),
            w.window_minutes,
            w.used_percent,
            w.reset_at,
            observed_at,
        ) {
            log::warn!(
                "[account_pool] could not save the quota reading of provider={provider_id}: {e}"
            );
        }
    }
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

/// Whether upstream refused the account outright for usage and its limit has
/// not yet reset.
pub fn is_limited(provider_id: &str, now: i64) -> bool {
    QUOTAS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider_id)
        .and_then(|q| q.limited_until)
        .is_some_and(|until| until > now)
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

/// When the account's weekly window resets: the longest account-wide window,
/// while it is running. `None` when no response has reported one or the last
/// reading's reset has passed.
fn weekly_reset_of(quota: &AccountQuota, now: i64) -> Option<i64> {
    quota
        .windows
        .iter()
        .filter(|w| w.limit_name.is_none())
        .max_by_key(|w| w.window_minutes)?
        .reset_at
        .filter(|reset| *reset > now)
}

/// The usage percentage at which rotation passes `provider` over: its own
/// switch-away limit when it has a valid one, otherwise the pool's.
pub fn threshold_for(provider: &Provider, pool_threshold: u8) -> u8 {
    provider
        .meta
        .as_ref()
        .and_then(|m| m.rotation_threshold_percent)
        .filter(|t| (THRESHOLD_MIN..=100).contains(t))
        .unwrap_or(pool_threshold)
}

/// The lowest switch-away limit, for the pool and for one account.
pub const THRESHOLD_MIN: u8 = 50;

/// How long an account the pool passed over is kept from taking an app back
/// by recovery: until this long has gone by without it being passed over
/// again. Recovery serves quota that would expire at a weekly reset, so an
/// hour's wait costs little of it.
const RECOVERY_HOLD_SECS: i64 = 3600;

/// When the pool last passed each account over for a reason that leaves it
/// able to serve other requests: a window or refusal scoped to one model, or
/// a failure that says something about the account. Unix seconds.
static PASSED_OVER: Lazy<Mutex<HashMap<String, i64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Notes that a request moved off `provider_id` after a failure that says
/// something about the account, so recovery does not move the app straight
/// back onto it.
pub fn note_passed_over(provider_id: &str) {
    note_passed_over_at(provider_id, chrono::Utc::now().timestamp());
}

fn note_passed_over_at(provider_id: &str, now: i64) {
    PASSED_OVER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider_id.to_string(), now);
}

fn passed_over_recently(provider_id: &str, now: i64) -> bool {
    PASSED_OVER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider_id)
        .is_some_and(|at| now - at < RECOVERY_HOLD_SECS)
}

/// Whether an account whose week ends at `challenger` takes the lead from the
/// current account, whose week ends at `current`: only when it ends more than
/// `SAME_RESET_SECS` sooner, or the current account has no running week.
fn takes_the_lead(challenger: Option<i64>, current: Option<i64>) -> bool {
    match (challenger, current) {
        (Some(challenger), Some(current)) => challenger + SAME_RESET_SECS < current,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Orders accounts for `model`. The switching order lists the app's current
/// account first, and it keeps the lead while it is not at its limit, unless
/// another account's weekly window resets more than an hour sooner: that one
/// leads, so its quota is used before it expires, and an account that went to
/// the back for its session window comes back to the front once that window
/// resets. An account passed over in the last hour for one model's quota, or
/// after a failure of its own, does not take the lead that way; it follows
/// the others. Other accounts not at their limit follow by weekly reset,
/// those with no running weekly window in queue order. Accounts at their
/// limit go last, in queue order. They stay in the list: when every account
/// is spent the request still goes out and upstream's own answer reaches the
/// client.
pub fn order_by_quota(
    providers: Vec<Provider>,
    model: Option<&str>,
    threshold_percent: u8,
) -> Vec<Provider> {
    let now = chrono::Utc::now().timestamp();
    let current = providers.first().map(|p| p.id.clone());
    let (mut fresh, spent): (Vec<_>, Vec<_>) = providers
        .into_iter()
        .partition(|p| !is_spent(&p.id, model, threshold_for(p, threshold_percent), now));
    // Spent for this model alone, the account can still serve requests for
    // others; noted, it does not take the app back on one of them.
    for p in &spent {
        if !is_spent(&p.id, None, threshold_for(p, threshold_percent), now) {
            note_passed_over_at(&p.id, now);
        }
    }
    let quotas = QUOTAS.read().unwrap_or_else(|e| e.into_inner());
    let reset = |p: &Provider| quotas.get(&p.id).and_then(|q| weekly_reset_of(q, now));
    fresh.sort_by_cached_key(|p| {
        let held = current.as_deref() != Some(p.id.as_str()) && passed_over_recently(&p.id, now);
        (held, reset(p).map_or((1, 0), |r| (0, r)))
    });
    if let Some(at) = fresh
        .iter()
        .position(|p| current.as_deref() == Some(p.id.as_str()))
    {
        if at > 0 && !takes_the_lead(reset(&fresh[0]), reset(&fresh[at])) {
            let kept = fresh.remove(at);
            fresh.insert(0, kept);
        }
    }
    drop(quotas);
    fresh.into_iter().chain(spent).collect()
}

/// Why rotation served another account ahead of `current`, for the switch
/// history: `current` was at its own switch-away limit or the pool's, or
/// another account's weekly window resets sooner.
pub fn rotation_reason(
    current: &Provider,
    model: Option<&str>,
    pool_threshold: u8,
) -> (SwitchReason, String) {
    let now = chrono::Utc::now().timestamp();
    let threshold = threshold_for(current, pool_threshold);
    if is_spent(&current.id, model, threshold, now) {
        let reason = if threshold == pool_threshold {
            "Near its usage limit".to_string()
        } else {
            format!("Reached its own {threshold}% switch-away limit")
        };
        (SwitchReason::Rotation, reason)
    } else {
        (
            SwitchReason::Recovered,
            "Another account's weekly limit resets sooner".to_string(),
        )
    }
}

/// Quota last seen for each account, for the UI.
pub fn quota_snapshot() -> HashMap<String, AccountQuota> {
    QUOTAS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_is_kept_once_per_whole_point_and_per_new_window() {
        let db = std::sync::Arc::new(crate::database::Database::memory().unwrap());
        keep_quota_history(db.clone());
        let db = QUOTA_HISTORY.get().unwrap();
        let id = "quota-history-test";
        let week = |used: f64, reset: i64| {
            record_windows(
                id,
                None,
                vec![QuotaWindow {
                    limit_name: None,
                    window_minutes: 10080,
                    used_percent: used,
                    reset_at: Some(reset),
                }],
            )
        };
        week(10.2, 1_000_000);
        week(10.8, 1_000_010);
        week(11.0, 1_000_020);
        week(0.0, 1_600_000);
        let conn = db.conn.lock().unwrap();
        let used: Vec<f64> = conn
            .prepare("SELECT used_percent FROM quota_readings WHERE provider_id = ?1 ORDER BY id")
            .unwrap()
            .query_map([id], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(used, vec![10.2, 11.0, 0.0]);
    }

    #[test]
    fn unsupported_country_is_advisory() {
        let status = observe(Ok("ip=1.2.3.4\nloc=CN\n".into()), None);
        assert_eq!(status.country.as_deref(), Some("CN"));
        assert!(status.warning.unwrap().contains("do not stop requests"));
    }

    #[test]
    fn a_country_change_is_reported() {
        let old = observe(Ok("ip=1.2.3.4\nloc=GB\n".into()), None);
        let status = observe(Ok("ip=2.3.4.5\nloc=US\n".into()), Some(&old));
        assert_eq!(status.country.as_deref(), Some("US"));
        assert!(status.warning.unwrap().contains("changed from GB to US"));
    }

    #[test]
    fn failed_lookup_keeps_last_country_and_reports_a_warning() {
        let old = observe(Ok("ip=203.0.113.10\nloc=GB\n".into()), None);
        for trace in [
            Err("connection reset".into()),
            Ok("ip=1.2.3.4\n".into()),
            Ok(String::new()),
        ] {
            let status = observe(trace, Some(&old));
            assert_eq!(status.country.as_deref(), Some("GB"));
            assert!(status.warning.unwrap().contains("do not stop requests"));
        }
    }

    fn observe(
        trace: Result<String, String>,
        previous: Option<&ProxyRouteStatus>,
    ) -> ProxyRouteStatus {
        exit_status(
            "test proxy".into(),
            "api.anthropic.com".into(),
            trace,
            &["CN".into()],
            previous,
        )
    }

    #[tokio::test]
    async fn region_observation_returns_before_a_dead_proxy_can_answer() {
        use crate::provider::{ProviderMeta, ProviderProxyConfig};
        let db = Database::memory().unwrap();
        let mut provider = Provider::with_id(
            "advisory-test".into(),
            "test".into(),
            serde_json::json!({}),
            None,
        );
        provider.meta = Some(ProviderMeta {
            proxy_config: Some(ProviderProxyConfig {
                enabled: true,
                proxy_type: Some("socks5".into()),
                proxy_host: Some("127.0.0.1".into()),
                proxy_port: Some(1),
                proxy_username: None,
                proxy_password: None,
            }),
            ..Default::default()
        });
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            ensure_exit_allowed(&db, &provider, "https://api.anthropic.com/cdn-cgi/trace"),
        )
        .await
        .expect("model request waited for region probe")
        .unwrap();
    }

    #[test]
    fn pool_config_defaults_warn_about_china_and_tolerate_old_saved_values() {
        let saved: AccountPoolConfig =
            serde_json::from_str(r#"{"enabled":true,"thresholdPercent":95}"#).unwrap();
        assert_eq!(saved.blocked_exit_countries, vec!["CN".to_string()]);
        assert!(!saved.require_proxy);
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
            usage_reset_at: HashMap::new(),
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

    fn session_and_week(id: &str, session_used: f64, session_reset: i64, week_reset: i64) {
        record_windows(
            id,
            None,
            vec![
                QuotaWindow {
                    limit_name: None,
                    window_minutes: 300,
                    used_percent: session_used,
                    reset_at: Some(session_reset),
                },
                QuotaWindow {
                    limit_name: None,
                    window_minutes: 10080,
                    used_percent: 40.0,
                    reset_at: Some(week_reset),
                },
            ],
        );
    }

    fn ids(providers: Vec<Provider>) -> Vec<String> {
        providers.into_iter().map(|p| p.id).collect()
    }

    #[test]
    fn the_account_whose_week_ends_first_leads_and_comes_back_after_its_session_resets() {
        let now = chrono::Utc::now().timestamp();
        let day = 86_400;
        let queue = || {
            ["ord-current", "ord-unread", "ord-week-ends"]
                .map(|id| Provider::with_id(id.into(), id.into(), serde_json::json!({}), None))
                .to_vec()
        };
        session_and_week("ord-current", 10.0, now + 3_600, now + 5 * day);
        session_and_week("ord-week-ends", 100.0, now + 3_600, now + day);
        assert_eq!(
            ids(order_by_quota(queue(), None, 98)),
            ["ord-current", "ord-unread", "ord-week-ends"],
            "its session window is spent, so it waits at the back"
        );
        assert_eq!(
            rotation_reason(&queue()[2], None, 98).1,
            "Near its usage limit"
        );

        session_and_week("ord-week-ends", 2.0, now + 5 * 3_600, now + day);
        assert_eq!(
            ids(order_by_quota(queue(), None, 98)),
            ["ord-week-ends", "ord-current", "ord-unread"],
            "its session window reset, and its week ends first"
        );
        assert_eq!(
            rotation_reason(&queue()[0], None, 98).0,
            SwitchReason::Recovered
        );
    }

    fn with_limit(id: &str, limit: Option<u8>) -> Provider {
        let mut provider = Provider::with_id(id.into(), id.into(), serde_json::json!({}), None);
        provider.meta = Some(crate::provider::ProviderMeta {
            rotation_threshold_percent: limit,
            ..Default::default()
        });
        provider
    }

    fn session_at(id: &str, used: f64, week_reset: i64) {
        record_windows(
            id,
            None,
            vec![QuotaWindow {
                limit_name: None,
                window_minutes: 10080,
                used_percent: used,
                reset_at: Some(week_reset),
            }],
        );
    }

    #[test]
    fn an_account_limit_below_the_pool_limit_moves_it_away_sooner() {
        let now = chrono::Utc::now().timestamp();
        let day = 86_400;
        // At 96% with its week ending first: the pool's 99% keeps it in front,
        // its own 90% sends it to the back.
        session_at("own-low-busy", 96.0, now + day);
        session_at("own-low-other", 0.0, now + 5 * day);
        let queue = |limit| {
            vec![
                with_limit("own-low-busy", limit),
                with_limit("own-low-other", None),
            ]
        };
        assert_eq!(
            ids(order_by_quota(queue(None), None, 99)),
            ["own-low-busy", "own-low-other"]
        );
        assert_eq!(
            ids(order_by_quota(queue(Some(90)), None, 99)),
            ["own-low-other", "own-low-busy"]
        );
        assert_eq!(
            rotation_reason(&with_limit("own-low-busy", Some(90)), None, 99),
            (
                SwitchReason::Rotation,
                "Reached its own 90% switch-away limit".to_string()
            )
        );
    }

    #[test]
    fn an_account_limit_above_the_pool_limit_keeps_it_in_use_longer() {
        let now = chrono::Utc::now().timestamp();
        let day = 86_400;
        session_at("own-high-busy", 96.0, now + day);
        session_at("own-high-other", 0.0, now + 5 * day);
        let queue = |limit| {
            vec![
                with_limit("own-high-busy", limit),
                with_limit("own-high-other", None),
            ]
        };
        assert_eq!(
            ids(order_by_quota(queue(None), None, 90)),
            ["own-high-other", "own-high-busy"]
        );
        assert_eq!(
            ids(order_by_quota(queue(Some(99)), None, 90)),
            ["own-high-busy", "own-high-other"]
        );
    }

    fn account(id: &str) -> Provider {
        Provider::with_id(id.into(), id.into(), serde_json::json!({}), None)
    }

    fn queue(ids: &[&str]) -> Vec<Provider> {
        ids.iter().map(|id| account(id)).collect()
    }

    #[test]
    fn a_lower_reading_of_the_same_window_is_an_older_one() {
        // Claude 2026-11-10: answers read 99, 98, 99 within seconds at a 99%
        // limit, and the account was passed over and taken back in turn.
        let now = chrono::Utc::now().timestamp();
        let reset = now + 4 * 86_400;
        session_at("flicker", 99.0, reset);
        session_at("flicker", 98.0, reset + 7);
        assert!(is_spent("flicker", None, 99, now));
        assert_eq!(quota_snapshot()["flicker"].windows[0].used_percent, 99.0);
        // A new window starts low.
        session_at("flicker", 1.0, reset + 7 * 86_400);
        assert!(!is_spent("flicker", None, 99, now));
    }

    #[test]
    fn a_reset_that_lowers_usage_in_the_same_window_is_taken() {
        // 2026-11-18: after a Fable reset, card 58cad6b9 read 76% account-wide
        // and 0% for Fable, its week ending at the same second, while the pool
        // kept 99% and passed it over.
        let now = chrono::Utc::now().timestamp();
        let reset = now + 4 * 86_400;
        let read = |week: f64, fable: f64| {
            record_windows(
                "fable-reset",
                Some("reset-test-fable"),
                vec![
                    QuotaWindow {
                        limit_name: None,
                        window_minutes: 10080,
                        used_percent: week,
                        reset_at: Some(reset),
                    },
                    QuotaWindow {
                        limit_name: Some("7d_oi".into()),
                        window_minutes: 10080,
                        used_percent: fable,
                        reset_at: Some(reset),
                    },
                ],
            )
        };
        let used = || {
            quota_snapshot()["fable-reset"]
                .windows
                .iter()
                .map(|w| w.used_percent)
                .collect::<Vec<_>>()
        };
        read(99.0, 49.0);
        assert!(is_spent("fable-reset", Some("reset-test-fable"), 99, now));
        read(76.0, 0.0);
        assert_eq!(used(), [76.0, 0.0]);
        assert!(!is_spent("fable-reset", Some("reset-test-fable"), 99, now));
        // An answer from before the reset, arriving after it, does not undo it.
        read(99.0, 49.0);
        assert_eq!(used(), [76.0, 0.0]);
        // Usage then climbs as before, and late answers stay ignored.
        read(77.0, 1.0);
        read(76.0, 0.0);
        assert_eq!(used(), [77.0, 1.0]);
    }

    #[test]
    fn only_a_drop_of_more_than_a_few_points_is_a_reset() {
        let reset = 1_000_000;
        let reading = |used: f64| QuotaWindow {
            limit_name: None,
            window_minutes: 10080,
            used_percent: used,
            reset_at: Some(reset + 30),
        };
        let take = |q: &mut AccountQuota, used: f64, now: i64| {
            let mut windows = vec![reading(used)];
            ignore_late_answers("drop-size", q, &mut windows, now);
            merge_windows(q, windows);
            q.windows[0].used_percent
        };
        let mut q = quota(99.0, Some(reset), None);
        assert_eq!(
            take(&mut q, 94.0, 1000),
            99.0,
            "five points is a late answer"
        );
        assert_eq!(take(&mut q, 93.0, 1000), 93.0, "six points is a reset");
        assert_eq!(take(&mut q, 99.0, 1000 + LATE_ANSWER_SECS - 1), 93.0);
        assert_eq!(take(&mut q, 99.0, 1000 + LATE_ANSWER_SECS), 99.0);
        // A reading of another window is not compared.
        let mut q = quota(99.0, Some(reset - 2 * 3_600), None);
        assert_eq!(take(&mut q, 10.0, 1000), 10.0);
        assert!(q.usage_reset_at.is_empty());
    }

    #[test]
    fn weeks_ending_within_an_hour_keep_the_current_account_in_front() {
        // Codex 2026-11-06: both weeks ended at 06:43, each reading's reset
        // seconds off the last, and the pool moved between them on every one.
        let now = chrono::Utc::now().timestamp();
        let reset = now + 5 * 86_400;
        for (a, b) in [(reset + 20, reset), (reset, reset + 20)] {
            session_at("same-week-a", 40.0, a);
            session_at("same-week-b", 60.0, b);
            assert_eq!(
                ids(order_by_quota(
                    queue(&["same-week-a", "same-week-b"]),
                    None,
                    98
                )),
                ["same-week-a", "same-week-b"]
            );
            assert_eq!(
                ids(order_by_quota(
                    queue(&["same-week-b", "same-week-a"]),
                    None,
                    98
                )),
                ["same-week-b", "same-week-a"]
            );
        }
        // A week ending more than an hour sooner takes the lead.
        session_at("same-week-b", 60.0, reset - 2 * 3_600);
        assert_eq!(
            ids(order_by_quota(
                queue(&["same-week-a", "same-week-b"]),
                None,
                98
            )),
            ["same-week-b", "same-week-a"]
        );
    }

    #[test]
    fn an_account_passed_over_for_one_model_does_not_take_the_app_back_for_another() {
        let now = chrono::Utc::now().timestamp();
        let day = 86_400;
        record_windows(
            "scoped-spent",
            Some("scoped-test-fable"),
            vec![
                QuotaWindow {
                    limit_name: None,
                    window_minutes: 10080,
                    used_percent: 50.0,
                    reset_at: Some(now + day),
                },
                QuotaWindow {
                    limit_name: Some("7d_oi".into()),
                    window_minutes: 10080,
                    used_percent: 99.0,
                    reset_at: Some(now + day),
                },
            ],
        );
        session_at("scoped-other", 10.0, now + 5 * day);
        // Its week ends first, so it leads while nothing has passed it over.
        assert_eq!(
            ids(order_by_quota(
                queue(&["scoped-other", "scoped-spent"]),
                Some("scoped-test-opus"),
                98
            )),
            ["scoped-spent", "scoped-other"]
        );
        // A Fable request passes it over and the app moves on.
        assert_eq!(
            ids(order_by_quota(
                queue(&["scoped-spent", "scoped-other"]),
                Some("scoped-test-fable"),
                98
            )),
            ["scoped-other", "scoped-spent"]
        );
        // An Opus request next does not take the app back.
        assert_eq!(
            ids(order_by_quota(
                queue(&["scoped-other", "scoped-spent"]),
                Some("scoped-test-opus"),
                98
            )),
            ["scoped-other", "scoped-spent"]
        );
        // An hour without being passed over, it leads again.
        note_passed_over_at("scoped-spent", now - RECOVERY_HOLD_SECS);
        assert_eq!(
            ids(order_by_quota(
                queue(&["scoped-other", "scoped-spent"]),
                Some("scoped-test-opus"),
                98
            )),
            ["scoped-spent", "scoped-other"]
        );
    }

    #[test]
    fn an_account_that_failed_follows_the_others_until_an_hour_has_passed() {
        let now = chrono::Utc::now().timestamp();
        let day = 86_400;
        session_at("failed-soonest", 10.0, now + day);
        session_at("failed-current", 10.0, now + 3 * day);
        session_at("failed-third", 10.0, now + 5 * day);
        note_passed_over("failed-soonest");
        assert_eq!(
            ids(order_by_quota(
                queue(&["failed-current", "failed-soonest", "failed-third"]),
                None,
                98
            )),
            ["failed-current", "failed-third", "failed-soonest"]
        );
        // When the current account is spent, the pool moves to one that has
        // not failed.
        session_at("failed-current", 99.0, now + 3 * day);
        assert_eq!(
            ids(order_by_quota(
                queue(&["failed-current", "failed-soonest", "failed-third"]),
                None,
                98
            )),
            ["failed-third", "failed-soonest", "failed-current"]
        );
        note_passed_over_at("failed-soonest", now - RECOVERY_HOLD_SECS);
        assert_eq!(
            ids(order_by_quota(
                queue(&["failed-current", "failed-soonest", "failed-third"]),
                None,
                98
            )),
            ["failed-soonest", "failed-third", "failed-current"]
        );
    }

    #[test]
    fn an_account_limit_outside_the_allowed_range_falls_back_to_the_pool() {
        assert_eq!(threshold_for(&with_limit("own-range", None), 98), 98);
        assert_eq!(threshold_for(&with_limit("own-range", Some(85)), 98), 85);
        assert_eq!(threshold_for(&with_limit("own-range", Some(10)), 98), 98);
        assert_eq!(threshold_for(&with_limit("own-range", Some(101)), 98), 98);
    }
}
