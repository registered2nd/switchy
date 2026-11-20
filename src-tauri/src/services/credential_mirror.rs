//! Background reconciler that keeps Claude Code's OAuth login (`claudeAiOauth`
//! in `~/.claude/.credentials.json`) consistent across two independent installs
//! (typically Windows + WSL) without letting them race each other to death.
//!
//! Why this exists: Claude Code rotates the OAuth refresh token on every refresh
//! (single-use, server-enforced). When both sides hold the *same* refresh token,
//! whichever side refreshes first invalidates the other's. A long-lived session
//! re-reads `.credentials.json` per request, so writing a valid bundle back into
//! a side that lost the race lets it recover — this is exactly what a manual
//! Switchy account-switch does by hand. This watcher automates it.
//!
//! Design (supersedes the earlier one-way live -> mirror copy, which spread a
//! dead/blanked file outward when the *mirror* side won the race, killing both):
//!
//!   * Bidirectional. The freshest valid login flows to the stale side, in
//!     whichever direction it needs to.
//!   * Health-aware. A blanked / unparseable / dead bundle is NEVER propagated;
//!     only a bundle with a non-empty access + refresh token can win.
//!   * Account-guarded. Two *different accounts* (e.g. mid-switch, or a
//!     deliberate per-machine login) are left alone — freshness only decides a
//!     winner when both sides are the same account (matched by oauthAccount
//!     UUID). This is what makes it impossible to fight a manual switch.
//!   * Surgical. Only the `claudeAiOauth` block is copied; each machine keeps
//!     its own `mcpOAuth` and any other per-machine keys.
//!
//! Detection is event-driven on the local side (`notify`) plus a periodic poll
//! for the mirror side, because `notify` cannot reliably watch a `\\wsl$\` path.
//!
//! Keeping both installs on one login gives that login two renewers, each
//! renewing once under five minutes are left, which would post the same
//! refresh token together. While both hold the same login, the card holding
//! it renews it in the five minutes before that and writes the renewal to both
//! (`claude_pool::renew_shared_login`), so neither install reaches its window.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;

use crate::error::AppError;
use crate::services::claude_account::{paths, store};

/// Event-burst coalesce window for the local atomic-write (tmp + rename emits
/// several events back-to-back).
const COALESCE_WINDOW: Duration = Duration::from_millis(150);

/// How often to poll when no local event fires — this is the path that catches
/// a refresh performed by the *mirror* side (WSL), which `notify` can't see.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// The cards, for finding the one that holds a login both installs hold.
static DB: std::sync::OnceLock<std::sync::Arc<crate::database::Database>> =
    std::sync::OnceLock::new();

/// Spawn the reconciler on a dedicated background thread. Safe to call once at
/// app startup; no-op while no mirror dir is configured.
pub fn start(db: std::sync::Arc<crate::database::Database>) {
    let _ = DB.set(db);
    let _ = std::thread::Builder::new()
        .name("cred-mirror".to_string())
        .spawn(|| {
            if let Err(e) = run() {
                log::warn!("[credential_mirror] watcher exited: {e}");
            }
        });
}

/// Same reconciler for Codex's ChatGPT login (`tokens` in `~/.codex/auth.json`),
/// which rotates its refresh token the same way. After each pass a sign-in or
/// renewal made on either side is filed with its card straight away, whichever
/// account the other side holds.
pub fn start_codex(db: std::sync::Arc<crate::database::Database>) {
    let _ = std::thread::Builder::new()
        .name("codex-cred-mirror".to_string())
        .spawn(move || {
            let reconcile_and_file = move || {
                reconcile_codex();
                crate::proxy::codex_pool::file_live_login(&db);
            };
            if let Err(e) = run_watch(
                crate::codex_config::get_codex_auth_path(),
                reconcile_and_file,
                "credential_mirror/codex",
            ) {
                log::warn!("[credential_mirror/codex] watcher exited: {e}");
            }
        });
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_watch(
        paths::live_credentials_path(),
        reconcile,
        "credential_mirror",
    )
}

fn run_watch(
    live: std::path::PathBuf,
    reconcile: impl Fn(),
    tag: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let watch_dir = match live.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(()),
    };

    if !watch_dir.exists() {
        log::info!(
            "[{tag}] watch dir missing, skipping: {}",
            watch_dir.display()
        );
        return Ok(());
    }

    // Catch up immediately at startup (either side may be stale).
    reconcile();

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Config::default())?;
    watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

    log::info!(
        "[{tag}] bidirectional reconcile active on {} (poll {}s)",
        watch_dir.display(),
        POLL_INTERVAL.as_secs()
    );

    let live_filename = live.file_name().map(|s| s.to_owned());

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(event)) => {
                if !is_relevant(&event, live_filename.as_deref()) {
                    continue;
                }
                // Coalesce the atomic-write event burst, then drain.
                std::thread::sleep(COALESCE_WINDOW);
                while rx.try_recv().is_ok() {}
                reconcile();
            }
            Ok(Err(e)) => {
                log::warn!("[{tag}] watch error: {e}");
            }
            // Periodic tick: the only way to notice a refresh the mirror side
            // performed on its own (notify cannot watch `\\wsl$\`).
            Err(mpsc::RecvTimeoutError::Timeout) => reconcile(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

fn is_relevant(event: &notify::Event, live_filename: Option<&OsStr>) -> bool {
    if !matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
    ) {
        return false;
    }
    let Some(name) = live_filename else {
        return false;
    };
    event.paths.iter().any(|p| p.file_name() == Some(name))
}

// =====================================================================
//  reconcile (IO) — reads both sides, decides, propagates
// =====================================================================

fn reconcile() {
    let Some(mirror_dir) = crate::settings::get_claude_mirror_override_dir() else {
        return;
    };
    // If the mirror target is unreachable (WSL down / not set up), do nothing
    // rather than act on a one-sided view.
    if !mirror_dir.exists() {
        return;
    }

    let live_path = paths::live_credentials_path();
    let mirror_path = mirror_dir.join(".credentials.json");

    let mut live = load_side(&live_path);
    let mut mirror = load_side(&mirror_path);
    note_dropped_and_refused(&mut live, &mut mirror);
    renew_shared_login_when_due(&live, &mirror);

    // The account UUID lives in the (potentially large) `.claude.json`, so it
    // is read only when a decision needs it: a "both alive but different" tie,
    // or a heal, which is never made across accounts.
    if needs_account(&live, &mirror) || live.alive != mirror.alive {
        live.account = read_account_uuid(&paths::live_claude_config_path());
        mirror.account = read_account_uuid(&paths::mirror_claude_config_path(&mirror_dir));
    }

    let action = match decide(&live, &mirror) {
        Action::Propagate(_) if heals_across_accounts(&live, &mirror) => Action::Noop,
        action => action,
    };
    match action {
        Action::Noop => log_declined_heal(&live, &mirror),
        Action::Propagate(Side::Live) => {
            if let Some(oauth) = live.oauth.as_ref() {
                let why = if mirror.alive { "fresher" } else { "heal" };
                match propagate(oauth, &mirror_path) {
                    Ok(()) => log::info!(
                        "[credential_mirror] synced live -> mirror ({why}, expiresAt={})",
                        live.expires_at
                    ),
                    Err(e) => log::warn!("[credential_mirror] live -> mirror failed: {e}"),
                }
            }
        }
        Action::Propagate(Side::Mirror) => {
            if let Some(oauth) = mirror.oauth.as_ref() {
                let why = if live.alive { "fresher" } else { "heal" };
                match propagate(oauth, &live_path) {
                    Ok(()) => log::info!(
                        "[credential_mirror] synced mirror -> live ({why}, expiresAt={})",
                        mirror.expires_at
                    ),
                    Err(e) => log::warn!("[credential_mirror] mirror -> live failed: {e}"),
                }
            }
        }
    }
}

/// A renewal ahead of both installs is in flight.
static RENEWING_SHARED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The refresh token of the login both sides hold, once it is due to be
/// renewed ahead of both installs.
fn shared_login_due(live: &SideState, mirror: &SideState, now_ms: i64) -> Option<String> {
    let token = live.refresh_token()?;
    let shared = live.alive && mirror.alive && mirror.refresh_token() == Some(token);
    (shared && crate::proxy::claude_pool::renews_ahead_of_installs(live.expires_at - now_ms))
        .then(|| token.to_string())
}

/// Starts the renewal of the login both sides hold when it is due, one at a
/// time; it runs on the async runtime while this thread goes on reconciling.
fn renew_shared_login_when_due(live: &SideState, mirror: &SideState) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let Some(token) = shared_login_due(live, mirror, now_ms) else {
        return;
    };
    let Some(db) = DB.get().cloned() else {
        return;
    };
    use std::sync::atomic::Ordering;
    if RENEWING_SHARED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        crate::proxy::claude_pool::renew_shared_login(&db, &token).await;
        RENEWING_SHARED.store(false, Ordering::SeqCst);
    });
}

/// The refresh token each side held the last time it was seen alive:
/// `[live, mirror]`.
static LAST_ALIVE: std::sync::Mutex<[Option<String>; 2]> = std::sync::Mutex::new([None, None]);
/// The dropped login a heal was last declined for, so it is logged once.
static DECLINED_HEAL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Fills in what each side dropped and whether its token was refused, then
/// records the token of each side that is alive now.
fn note_dropped_and_refused(live: &mut SideState, mirror: &mut SideState) {
    let mut last = LAST_ALIVE.lock().unwrap_or_else(|e| e.into_inner());
    for (index, side) in [&mut *live, &mut *mirror].into_iter().enumerate() {
        side.dropped = last[index].clone();
        side.refused = side
            .refresh_token()
            .is_some_and(crate::proxy::claude_pool::refresh_token_was_refused);
        if side.alive {
            last[index] = side.refresh_token().map(str::to_string);
        }
    }
}

fn log_declined_heal(live: &SideState, mirror: &SideState) {
    let (source, target, from, to) = match (live.alive, mirror.alive) {
        (true, false) => (live, mirror, "live", "mirror"),
        (false, true) => (mirror, live, "mirror", "live"),
        _ => return,
    };
    let why = if heals_across_accounts(live, mirror) {
        "is signed in to another account"
    } else if restores_dropped_login(source, target) {
        "dropped this login after Anthropic refused it; it needs a new sign-in"
    } else {
        return;
    };
    let token = source.refresh_token().map(str::to_string);
    let mut declined = DECLINED_HEAL.lock().unwrap_or_else(|e| e.into_inner());
    if *declined != token {
        *declined = token;
        log::info!(
            "[credential_mirror] not copying {from} -> {to} (expiresAt={}): {to} {why}",
            source.expires_at
        );
    }
}

/// A heal onto a side signed in to another account would hand that install
/// a login it does not show, and the card of the account that side names
/// would take it as its own: one login, then renewed through two cards.
fn heals_across_accounts(live: &SideState, mirror: &SideState) -> bool {
    live.alive != mirror.alive
        && matches!((&live.account, &mirror.account), (Some(a), Some(b)) if a != b)
}

/// Read the `claudeAiOauth` block from a target file, replace it with `oauth`,
/// and write the result back atomically — preserving every other top-level key
/// (notably the per-machine `mcpOAuth`).
pub(crate) fn propagate(oauth: &Value, to_path: &Path) -> Result<(), AppError> {
    let mut root = match fs::read(to_path) {
        Ok(b) => serde_json::from_slice::<Value>(&b)
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };
    if !root.is_object() {
        root = Value::Object(serde_json::Map::new());
    }
    root.as_object_mut()
        .expect("root is object")
        .insert("claudeAiOauth".to_string(), oauth.clone());

    let bytes =
        serde_json::to_vec_pretty(&root).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(to_path, &bytes)
}

pub(crate) fn read_account_uuid(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let root: Value = serde_json::from_slice(&bytes).ok()?;
    root.get("oauthAccount")?
        .get("accountUuid")?
        .as_str()
        .map(str::to_string)
}

// =====================================================================
//  Codex — the same decision over `tokens` in `auth.json`
// =====================================================================

/// A Codex side: the shared verdict inputs, plus whether that install is
/// deliberately on an API key rather than a ChatGPT login.
#[derive(Debug, Default, Clone)]
struct CodexSide {
    state: SideState,
    root: Option<Value>,
    api_key: bool,
}

fn load_codex_side(path: &Path) -> CodexSide {
    let Ok(bytes) = fs::read(path) else {
        return CodexSide::default();
    };
    let Ok(root) = serde_json::from_slice::<Value>(&bytes) else {
        return CodexSide::default();
    };
    codex_side_from_root(root)
}

fn codex_side_from_root(root: Value) -> CodexSide {
    let api_key = root
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let login = crate::services::codex_account::inspect(&root);
    let state = SideState {
        oauth: root.get("tokens").cloned(),
        alive: login.as_ref().map(|l| l.alive).unwrap_or(false),
        expires_at: login.as_ref().map(|l| l.last_refresh).unwrap_or(0),
        account: login
            .as_ref()
            .and_then(|l| l.account_key().map(str::to_string)),
        ..SideState::default()
    };
    CodexSide {
        state,
        root: Some(root),
        api_key,
    }
}

pub(crate) fn reconcile_codex() {
    let Some(mirror_dir) = crate::settings::get_codex_mirror_override_dir() else {
        return;
    };
    if !mirror_dir.exists() {
        return;
    }

    let live_path = crate::codex_config::get_codex_auth_path();
    let mirror_path = mirror_dir.join("auth.json");

    let live = load_codex_side(&live_path);
    let mirror = load_codex_side(&mirror_path);

    // An install on an API key is not on a ChatGPT login at all; healing it
    // with the other side's tokens would change what it authenticates as.
    if live.api_key || mirror.api_key {
        return;
    }

    let copy = |from: &CodexSide, to: &Path, why: &str, direction: &str| {
        let Some(root) = from.root.as_ref() else {
            return;
        };
        match propagate_codex(root, to) {
            Ok(()) => log::info!(
                "[credential_mirror/codex] synced {direction} ({why}, lastRefresh={})",
                from.state.expires_at
            ),
            Err(e) => log::warn!("[credential_mirror/codex] {direction} failed: {e}"),
        }
    };

    match decide(&live.state, &mirror.state) {
        Action::Noop => {}
        Action::Propagate(Side::Live) => {
            let why = if mirror.state.alive {
                "fresher"
            } else {
                "heal"
            };
            copy(&live, &mirror_path, why, "live -> mirror");
        }
        Action::Propagate(Side::Mirror) => {
            let why = if live.state.alive { "fresher" } else { "heal" };
            copy(&mirror, &live_path, why, "mirror -> live");
        }
    }
}

/// Replace `tokens` + `last_refresh` in the target `auth.json` with the
/// source's, keeping every other key the target has.
fn propagate_codex(source_root: &Value, to_path: &Path) -> Result<(), AppError> {
    let target_root = match fs::read(to_path) {
        Ok(b) => serde_json::from_slice::<Value>(&b)
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };
    let merged = crate::services::codex_account::transplant_login(&target_root, source_root);
    let bytes =
        serde_json::to_vec_pretty(&merged).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(to_path, &bytes)
}

// =====================================================================
//  decision logic (pure) — unit-tested without touching the filesystem
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Live,
    Mirror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Noop,
    /// Copy the `claudeAiOauth` from this side onto the other side.
    Propagate(Side),
}

#[derive(Debug, Default, Clone)]
struct SideState {
    /// Parsed `claudeAiOauth` block, if the file parsed and carried one.
    oauth: Option<Value>,
    /// Non-empty access AND refresh token present (i.e. usable / recoverable).
    alive: bool,
    /// `expiresAt` (ms) of the access token; later == more recently refreshed.
    expires_at: i64,
    /// Owning account UUID (from `.claude.json`); only loaded to break ties.
    account: Option<String>,
    /// The refresh token this side held the last time it was seen alive. When
    /// Claude Code is refused a renewal it clears its login; the token it held
    /// then is the one it dropped.
    dropped: Option<String>,
    /// Anthropic refused this side's refresh token (the proxy recorded it).
    refused: bool,
}

impl SideState {
    fn refresh_token(&self) -> Option<&str> {
        self.oauth
            .as_ref()?
            .get("refreshToken")?
            .as_str()
            .filter(|s| !s.is_empty())
    }
}

/// Copying `source` onto `cleared`, a side whose login is gone, would put back
/// the login Claude Code dropped there after it was refused: the same refresh
/// token `cleared` last held, or one Anthropic refused. That brings back no
/// login; it sends every session on that side to renew a dead token again.
fn restores_dropped_login(source: &SideState, cleared: &SideState) -> bool {
    source.refused
        || source
            .refresh_token()
            .is_some_and(|token| cleared.dropped.as_deref() == Some(token))
}

fn load_side(path: &Path) -> SideState {
    let Ok(bytes) = fs::read(path) else {
        return SideState::default();
    };
    let Ok(root) = serde_json::from_slice::<Value>(&bytes) else {
        return SideState::default();
    };
    side_state_from_root(&root)
}

/// Split out from `load_side` so the parse/health logic is unit-testable.
fn side_state_from_root(root: &Value) -> SideState {
    let oauth = root
        .get("claudeAiOauth")
        .cloned()
        .or_else(|| root.get("accessToken").map(|_| root.clone()));
    let alive = oauth
        .as_ref()
        .map(|o| nonempty(o, "accessToken") && nonempty(o, "refreshToken"))
        .unwrap_or(false);
    let expires_at = oauth
        .as_ref()
        .and_then(|o| o.get("expiresAt"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    SideState {
        oauth,
        alive,
        expires_at,
        ..SideState::default()
    }
}

/// The reconciler's two verdicts on a credentials blob, exposed so the
/// per-provider snapshot store can be held to the same rules: `(alive,
/// access-token expiry in ms)`. Alive means both tokens are present and
/// non-empty; the expiry orders two bundles of the same account, later being
/// the more recently refreshed one.
pub(crate) fn credential_health(root: &Value) -> (bool, i64) {
    let state = side_state_from_root(root);
    (state.alive, state.expires_at)
}

fn nonempty(obj: &Value, key: &str) -> bool {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// True only when the tie-breaker (account UUID) is actually needed: both sides
/// are alive and carry different logins.
fn needs_account(live: &SideState, mirror: &SideState) -> bool {
    live.alive && mirror.alive && live.oauth != mirror.oauth
}

/// Decide which way (if any) the login should flow. Pure.
fn decide(live: &SideState, mirror: &SideState) -> Action {
    // Already consistent.
    if live.alive && mirror.alive && live.oauth == mirror.oauth {
        return Action::Noop;
    }

    match (live.alive, mirror.alive) {
        // Heal: exactly one side has a usable login -> copy it to the other,
        // unless it is the very login the other side dropped.
        (true, false) if restores_dropped_login(live, mirror) => Action::Noop,
        (false, true) if restores_dropped_login(mirror, live) => Action::Noop,
        (true, false) => Action::Propagate(Side::Live),
        (false, true) => Action::Propagate(Side::Mirror),
        // Nothing usable anywhere — only `/login` can fix this.
        (false, false) => Action::Noop,
        // Both alive but different. Only break the tie when we are certain it's
        // the SAME account (a refresh rotation), never across accounts (a switch
        // in progress, or a deliberate per-machine login).
        (true, true) => match (&live.account, &mirror.account) {
            (Some(a), Some(b)) if a == b => {
                if live.expires_at > mirror.expires_at {
                    Action::Propagate(Side::Live)
                } else if mirror.expires_at > live.expires_at {
                    Action::Propagate(Side::Mirror)
                } else {
                    Action::Noop
                }
            }
            _ => Action::Noop,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn oauth(access: &str, refresh: &str, expires_at: i64) -> Value {
        json!({
            "claudeAiOauth": {
                "accessToken": access,
                "refreshToken": refresh,
                "expiresAt": expires_at,
                "scopes": ["user:inference"],
                "subscriptionType": "max"
            }
        })
    }

    fn alive_side(expires_at: i64, account: Option<&str>) -> SideState {
        let mut s = side_state_from_root(&oauth("AAA", "RRR", expires_at));
        s.account = account.map(str::to_string);
        s
    }

    #[test]
    fn alive_detects_blanked_refresh_token() {
        let dead = side_state_from_root(&oauth("AAA", "", 100));
        assert!(!dead.alive);
        let live = side_state_from_root(&oauth("AAA", "RRR", 100));
        assert!(live.alive);
    }

    #[test]
    fn alive_requires_access_token_too() {
        let dead = side_state_from_root(&oauth("", "RRR", 100));
        assert!(!dead.alive);
    }

    #[test]
    fn missing_or_unparseable_is_not_alive() {
        let empty = side_state_from_root(&json!({}));
        assert!(!empty.alive);
        assert!(empty.oauth.is_none());
    }

    #[test]
    fn identical_logins_do_nothing() {
        let a = alive_side(100, Some("acct-1"));
        let b = alive_side(100, Some("acct-1"));
        assert_eq!(decide(&a, &b), Action::Noop);
    }

    #[test]
    fn heals_dead_mirror_from_live() {
        let live = alive_side(100, None);
        let mirror = side_state_from_root(&oauth("AAA", "", 100)); // blanked
        assert_eq!(decide(&live, &mirror), Action::Propagate(Side::Live));
    }

    #[test]
    fn heals_dead_live_from_mirror() {
        let live = side_state_from_root(&json!({})); // missing
        let mirror = alive_side(100, None);
        assert_eq!(decide(&live, &mirror), Action::Propagate(Side::Mirror));
    }

    #[test]
    fn both_dead_does_nothing() {
        let live = side_state_from_root(&oauth("", "", 0));
        let mirror = side_state_from_root(&json!({}));
        assert_eq!(decide(&live, &mirror), Action::Noop);
    }

    #[test]
    fn same_account_fresher_live_wins() {
        let live = alive_side(200, Some("acct-1"));
        let mut mirror = alive_side(100, Some("acct-1"));
        // make tokens differ so it isn't the identical short-circuit
        mirror.oauth = Some(oauth("BBB", "SSS", 100));
        assert!(needs_account(&live, &mirror));
        assert_eq!(decide(&live, &mirror), Action::Propagate(Side::Live));
    }

    #[test]
    fn same_account_fresher_mirror_wins() {
        let mut live = alive_side(100, Some("acct-1"));
        live.oauth = Some(oauth("BBB", "SSS", 100));
        let mirror = alive_side(200, Some("acct-1"));
        assert_eq!(decide(&live, &mirror), Action::Propagate(Side::Mirror));
    }

    #[test]
    fn different_accounts_are_left_alone() {
        // This is the manual-switch guard: never revert a switch in progress.
        let mut live = alive_side(100, Some("acct-apple"));
        live.oauth = Some(oauth("BBB", "SSS", 100));
        let mirror = alive_side(999, Some("acct-gmail"));
        assert_eq!(decide(&live, &mirror), Action::Noop);
    }

    #[test]
    fn unknown_account_is_left_alone() {
        let mut live = alive_side(100, None);
        live.oauth = Some(oauth("BBB", "SSS", 100));
        let mirror = alive_side(999, None);
        assert_eq!(decide(&live, &mirror), Action::Noop);
    }

    #[test]
    fn same_account_equal_expiry_is_ambiguous_noop() {
        let mut live = alive_side(100, Some("acct-1"));
        live.oauth = Some(oauth("BBB", "SSS", 100));
        let mirror = alive_side(100, Some("acct-1"));
        // tokens differ, same expiry -> can't tell who's fresher -> leave alone
        assert_eq!(decide(&live, &mirror), Action::Noop);
    }

    /// The side Claude Code cleared after Anthropic refused its renewal.
    fn cleared_side(dropped: Option<&str>) -> SideState {
        let mut s = side_state_from_root(&oauth("", "", 0));
        s.dropped = dropped.map(str::to_string);
        s
    }

    #[test]
    fn the_login_claude_code_dropped_is_not_copied_back() {
        // Both sides held RRR; Claude Code on Windows was refused it and
        // cleared it. WSL's copy is the same dead login.
        let live = cleared_side(Some("RRR"));
        let mirror = alive_side(100, Some("acct-1"));
        assert_eq!(decide(&live, &mirror), Action::Noop);
        // And the other way round.
        assert_eq!(decide(&mirror, &live), Action::Noop);
    }

    #[test]
    fn a_refused_login_is_not_copied_onto_a_cleared_side() {
        let live = cleared_side(None);
        let mut mirror = alive_side(100, None);
        mirror.refused = true;
        assert_eq!(decide(&live, &mirror), Action::Noop);
    }

    #[test]
    fn a_newer_login_still_heals_the_side_that_lost_the_race() {
        // WSL renewed RRR to SSS first; Windows was refused RRR and cleared it.
        let live = cleared_side(Some("RRR"));
        let mut mirror = alive_side(200, None);
        mirror.oauth = Some(oauth("BBB", "SSS", 200)["claudeAiOauth"].clone());
        assert_eq!(decide(&live, &mirror), Action::Propagate(Side::Mirror));
    }

    #[test]
    #[serial_test::serial]
    fn each_side_remembers_the_token_it_last_held_alive() {
        *LAST_ALIVE.lock().unwrap() = [None, None];
        let mut live = alive_side(100, None);
        let mut mirror = alive_side(100, None);
        note_dropped_and_refused(&mut live, &mut mirror);
        assert_eq!(live.dropped, None);

        // Windows clears its login; WSL still holds the same one.
        let mut live = cleared_side(None);
        let mut mirror = alive_side(100, None);
        note_dropped_and_refused(&mut live, &mut mirror);
        assert_eq!(live.dropped.as_deref(), Some("RRR"));
        assert_eq!(decide(&live, &mirror), Action::Noop);
        *LAST_ALIVE.lock().unwrap() = [None, None];
    }

    #[test]
    fn needs_account_only_when_both_alive_and_differ() {
        let a = alive_side(100, None);
        let b = alive_side(100, None); // identical -> no
        assert!(!needs_account(&a, &b));
        let dead = side_state_from_root(&json!({}));
        assert!(!needs_account(&a, &dead)); // one dead -> no
    }

    #[test]
    fn a_heal_is_not_made_onto_a_side_signed_in_to_another_account() {
        let mirror = alive_side(100, Some("acct-B"));
        let mut other_account = cleared_side(None);
        other_account.account = Some("acct-A".to_string());
        assert!(heals_across_accounts(&other_account, &mirror));
        assert!(heals_across_accounts(&mirror, &other_account));

        // The same account heals, and so does an install whose account is not
        // known.
        let mut same_account = cleared_side(None);
        same_account.account = Some("acct-B".to_string());
        assert!(!heals_across_accounts(&same_account, &mirror));
        assert!(!heals_across_accounts(&cleared_side(None), &mirror));
        // Two live logins are a tie, not a heal.
        assert!(!heals_across_accounts(
            &alive_side(100, Some("acct-A")),
            &mirror
        ));
    }

    #[test]
    fn a_login_both_sides_hold_is_due_for_renewal_five_to_ten_minutes_ahead() {
        let now = 1_000_000_000_000;
        let minutes = |m: i64| now + m * 60_000;
        let side = |expires_at| alive_side(expires_at, None);
        assert_eq!(
            shared_login_due(&side(minutes(8)), &side(minutes(8)), now).as_deref(),
            Some("RRR")
        );
        assert_eq!(shared_login_due(&side(minutes(11)), &side(minutes(11)), now), None);
        assert_eq!(shared_login_due(&side(minutes(4)), &side(minutes(4)), now), None);

        // Two different logins are not one login with two renewers.
        let mut other = side(minutes(8));
        other.oauth = Some(oauth("BBB", "SSS", minutes(8))["claudeAiOauth"].clone());
        assert_eq!(shared_login_due(&side(minutes(8)), &other, now), None);
        // A side with no login holds nothing.
        assert_eq!(
            shared_login_due(&side(minutes(8)), &cleared_side(None), now),
            None
        );
    }
}
