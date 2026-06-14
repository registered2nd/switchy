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

/// Spawn the reconciler on a dedicated background thread. Safe to call once at
/// app startup; no-op while no mirror dir is configured.
pub fn start() {
    let _ = std::thread::Builder::new()
        .name("cred-mirror".to_string())
        .spawn(|| {
            if let Err(e) = run() {
                log::warn!("[credential_mirror] watcher exited: {e}");
            }
        });
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let live = paths::live_credentials_path();
    let watch_dir = match live.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(()),
    };

    if !watch_dir.exists() {
        log::info!(
            "[credential_mirror] watch dir missing, skipping: {}",
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
        "[credential_mirror] bidirectional reconcile active on {} (poll {}s)",
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
                log::warn!("[credential_mirror] watch error: {e}");
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

    // The account UUID is only needed to break a "both alive but different" tie,
    // and lives in the (potentially large) `.claude.json` — so only read it when
    // a tie actually has to be broken.
    if needs_account(&live, &mirror) {
        live.account = read_account_uuid(&paths::live_claude_config_path());
        mirror.account = read_account_uuid(&paths::mirror_claude_config_path(&mirror_dir));
    }

    match decide(&live, &mirror) {
        Action::Noop => {}
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

/// Read the `claudeAiOauth` block from a target file, replace it with `oauth`,
/// and write the result back atomically — preserving every other top-level key
/// (notably the per-machine `mcpOAuth`).
fn propagate(oauth: &Value, to_path: &Path) -> Result<(), AppError> {
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

    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(to_path, &bytes)
}

fn read_account_uuid(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let root: Value = serde_json::from_slice(&bytes).ok()?;
    root.get("oauthAccount")?
        .get("accountUuid")?
        .as_str()
        .map(str::to_string)
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
        account: None,
    }
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
        // Heal: exactly one side has a usable login -> copy it to the other.
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

    #[test]
    fn needs_account_only_when_both_alive_and_differ() {
        let a = alive_side(100, None);
        let b = alive_side(100, None); // identical -> no
        assert!(!needs_account(&a, &b));
        let dead = side_state_from_root(&json!({}));
        assert!(!needs_account(&a, &dead)); // one dead -> no
    }
}
