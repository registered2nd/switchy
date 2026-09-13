//! Integration tests for the claude_account service (capture, clear, swap).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use serial_test::serial;
use tempfile::TempDir;

use super::*;
use crate::database::Database;
use crate::provider::{Provider, ProviderMeta};
use crate::store::AppState;

// ---------- test harness ----------

struct ScopedHome {
    dir: TempDir,
    prev_test_home: Option<String>,
    prev_home: Option<String>,
    prev_userprofile: Option<String>,
    prev_mirror: Option<String>,
}

impl ScopedHome {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let prev_test_home = env::var("SWITCHY_TEST_HOME").ok();
        let prev_home = env::var("HOME").ok();
        let prev_userprofile = env::var("USERPROFILE").ok();
        env::set_var("SWITCHY_TEST_HOME", dir.path());
        env::set_var("HOME", dir.path());
        env::set_var("USERPROFILE", dir.path());
        let prev_mirror = env::var("SWITCHY_TEST_CLAUDE_MIRROR").ok();
        // Clear any globally configured mirror for tests unless explicitly set.
        clear_mirror_setting();
        Self {
            dir,
            prev_test_home,
            prev_home,
            prev_userprofile,
            prev_mirror,
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn claude_dir(&self) -> PathBuf {
        let p = self.path().join(".claude");
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// The file Claude Code reads: sibling of the config dir, not inside it.
    fn live_config(&self) -> PathBuf {
        self.path().join(".claude.json")
    }

    fn live_credentials(&self) -> PathBuf {
        self.claude_dir().join(".credentials.json")
    }
}

/// A credentials blob shaped like the real one: both tokens present, with an
/// access-token expiry the freshness rules can order.
fn credentials_blob(token: &str, expires_at: i64) -> Value {
    json!({
        "claudeAiOauth": {
            "accessToken": format!("access-{token}"),
            "refreshToken": format!("refresh-{token}"),
            "expiresAt": expires_at,
            "subscriptionType": "max",
        }
    })
}

impl Drop for ScopedHome {
    fn drop(&mut self) {
        match &self.prev_test_home {
            Some(v) => env::set_var("SWITCHY_TEST_HOME", v),
            None => env::remove_var("SWITCHY_TEST_HOME"),
        }
        match &self.prev_home {
            Some(v) => env::set_var("HOME", v),
            None => env::remove_var("HOME"),
        }
        match &self.prev_userprofile {
            Some(v) => env::set_var("USERPROFILE", v),
            None => env::remove_var("USERPROFILE"),
        }
        let _ = &self.prev_mirror;
        clear_mirror_setting();
    }
}

fn clear_mirror_setting() {
    let _ = crate::settings::set_claude_mirror_config_dir(None);
}

fn set_mirror(dir: &Path) {
    crate::settings::set_claude_mirror_config_dir(Some(dir.to_path_buf())).expect("set mirror dir");
}

fn make_state() -> AppState {
    let db = Arc::new(Database::memory().expect("memory db"));
    AppState::new(db)
}

fn official_provider(id: &str) -> Provider {
    Provider {
        id: id.to_string(),
        name: format!("Official {id}"),
        settings_config: json!({ "env": {} }),
        website_url: None,
        category: Some("official".to_string()),
        created_at: Some(1),
        sort_index: Some(0),
        notes: None,
        meta: Some(ProviderMeta::default()),
        icon: None,
        icon_color: None,
        in_failover_queue: false,
    }
}

fn write_live_files(home: &ScopedHome, account_uuid: &str, email: &str) {
    write_live_files_at(home, account_uuid, email, 1_000);
}

fn write_live_files_at(home: &ScopedHome, account_uuid: &str, email: &str, expires_at: i64) {
    fs::write(
        home.live_credentials(),
        credentials_blob(account_uuid, expires_at).to_string(),
    )
    .unwrap();
    fs::write(
        home.live_config(),
        json!({
            "oauthAccount": {
                "accountUuid": account_uuid,
                "emailAddress": email,
                "organizationName": "Acme",
            },
            "projects": { "p1": {} },
            "machineID": "machine-stays-put",
            "userID": format!("user-{account_uuid}"),
        })
        .to_string(),
    )
    .unwrap();
}

fn seed_provider(state: &AppState, p: &Provider) {
    state.db.save_provider(CLAUDE_APP_TYPE, p).expect("seed");
}

// ---------- capture ----------

#[test]
#[serial]
fn capture_happy_path_writes_snapshots_and_meta() {
    let home = ScopedHome::new();
    let state = make_state();
    let p = official_provider("p1");
    seed_provider(&state, &p);
    write_live_files(&home, "uuid-A", "alice@example.com");

    let outcome = capture(&state, "p1", false).expect("capture ok");
    match outcome {
        CaptureOutcome::Captured { identity } => {
            assert_eq!(identity.account_uuid, "uuid-A");
            assert_eq!(identity.email_address, "alice@example.com");
        }
        other => panic!("expected Captured, got {other:?}"),
    }

    assert!(paths::snapshot_credentials_path("p1").exists());
    assert!(paths::snapshot_oauth_account_path("p1").exists());

    let reloaded = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let captured = reloaded
        .meta
        .expect("meta")
        .captured_claude_account
        .expect("meta captured set");
    assert_eq!(captured.account_uuid, "uuid-A");
    assert_eq!(captured.email_address, "alice@example.com");
}

#[test]
#[serial]
fn capture_fails_when_credentials_missing() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    // Only write the claude config, no credentials file.
    fs::write(
        home.live_config(),
        json!({ "oauthAccount": { "accountUuid": "u", "emailAddress": "e@x" } }).to_string(),
    )
    .unwrap();

    let err = capture(&state, "p1", false).unwrap_err();
    match err {
        AppError::Localized { key, .. } => {
            assert_eq!(key, "claudeAccount.capture.error.credentials_missing");
        }
        other => panic!("expected localized credentials_missing, got {other:?}"),
    }
    assert!(!paths::snapshot_credentials_path("p1").exists());
}

#[test]
#[serial]
fn capture_fails_when_oauth_account_missing() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    fs::write(home.live_credentials(), "{}").unwrap();
    fs::write(home.live_config(), json!({ "projects": {} }).to_string()).unwrap();

    let err = capture(&state, "p1", false).unwrap_err();
    match err {
        AppError::Localized { key, .. } => {
            assert_eq!(key, "claudeAccount.capture.error.oauth_missing");
        }
        other => panic!("expected oauth_missing, got {other:?}"),
    }
}

#[test]
#[serial]
fn capture_fails_when_account_uuid_empty() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    fs::write(home.live_credentials(), "{}").unwrap();
    fs::write(
        home.live_config(),
        json!({ "oauthAccount": { "accountUuid": "", "emailAddress": "e@x" } }).to_string(),
    )
    .unwrap();

    let err = capture(&state, "p1", false).unwrap_err();
    match err {
        AppError::Localized { key, .. } => {
            assert_eq!(key, "claudeAccount.capture.error.oauth_missing");
        }
        other => panic!("expected oauth_missing, got {other:?}"),
    }
}

#[test]
#[serial]
fn capture_with_matching_uuid_overwrites_silently() {
    let home = ScopedHome::new();
    let state = make_state();
    let mut p = official_provider("p1");
    p.meta = Some(ProviderMeta {
        captured_claude_account: Some(crate::provider::CapturedClaudeAccountMeta {
            account_uuid: "uuid-A".into(),
            email_address: "alice@example.com".into(),
            captured_at: 1,
        }),
        ..Default::default()
    });
    seed_provider(&state, &p);
    write_live_files(&home, "uuid-A", "alice@example.com");

    let outcome = capture(&state, "p1", false).unwrap();
    assert!(matches!(outcome, CaptureOutcome::Captured { .. }));
}

#[test]
#[serial]
fn capture_with_different_uuid_needs_confirmation_without_force() {
    let home = ScopedHome::new();
    let state = make_state();
    let mut p = official_provider("p1");
    p.meta = Some(ProviderMeta {
        captured_claude_account: Some(crate::provider::CapturedClaudeAccountMeta {
            account_uuid: "uuid-A".into(),
            email_address: "alice@example.com".into(),
            captured_at: 1,
        }),
        ..Default::default()
    });
    seed_provider(&state, &p);
    write_live_files(&home, "uuid-B", "bob@example.com");

    let outcome = capture(&state, "p1", false).unwrap();
    match outcome {
        CaptureOutcome::NeedsConfirmation { existing, incoming } => {
            assert_eq!(existing.account_uuid, "uuid-A");
            assert_eq!(incoming.account_uuid, "uuid-B");
        }
        other => panic!("expected NeedsConfirmation, got {other:?}"),
    }
    // No files written.
    assert!(!paths::snapshot_credentials_path("p1").exists());
    // Meta unchanged.
    let reloaded = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded
            .meta
            .unwrap()
            .captured_claude_account
            .unwrap()
            .account_uuid,
        "uuid-A"
    );
}

#[test]
#[serial]
fn capture_with_different_uuid_overwrites_when_forced() {
    let home = ScopedHome::new();
    let state = make_state();
    let mut p = official_provider("p1");
    p.meta = Some(ProviderMeta {
        captured_claude_account: Some(crate::provider::CapturedClaudeAccountMeta {
            account_uuid: "uuid-A".into(),
            email_address: "alice@example.com".into(),
            captured_at: 1,
        }),
        ..Default::default()
    });
    seed_provider(&state, &p);
    write_live_files(&home, "uuid-B", "bob@example.com");

    let outcome = capture(&state, "p1", true).unwrap();
    match outcome {
        CaptureOutcome::Captured { identity } => assert_eq!(identity.account_uuid, "uuid-B"),
        other => panic!("expected Captured, got {other:?}"),
    }
    let reloaded = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded
            .meta
            .unwrap()
            .captured_claude_account
            .unwrap()
            .account_uuid,
        "uuid-B"
    );
}

// ---------- clear ----------

#[test]
#[serial]
fn clear_removes_files_and_meta_and_is_idempotent() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    assert!(paths::snapshot_dir("p1").exists());
    clear(&state, "p1").unwrap();
    assert!(!paths::snapshot_dir("p1").exists());

    let reloaded = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    assert!(reloaded
        .meta
        .and_then(|m| m.captured_claude_account)
        .is_none());

    // Second call is still Ok.
    clear(&state, "p1").unwrap();
}

#[test]
#[serial]
fn clear_does_not_touch_live_credentials() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    let cred_path = paths::live_credentials_path();
    let before = fs::read(&cred_path).unwrap();
    clear(&state, "p1").unwrap();
    let after = fs::read(&cred_path).unwrap();
    assert_eq!(before, after);
}

#[test]
#[serial]
fn clear_for_never_captured_provider_is_ok() {
    let _home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    clear(&state, "p1").unwrap();
}

// ---------- swap_if_captured ----------

#[test]
#[serial]
fn swap_skipped_for_non_official_provider() {
    let _home = ScopedHome::new();
    let state = make_state();
    let mut p = official_provider("p1");
    p.category = Some("custom".into());
    let outcome = swap_if_captured(&state, &p).unwrap();
    assert!(matches!(outcome, SwapOutcome::Skipped));
}

#[test]
#[serial]
fn swap_skipped_when_meta_absent() {
    let _home = ScopedHome::new();
    let state = make_state();
    let p = official_provider("p1");
    let outcome = swap_if_captured(&state, &p).unwrap();
    assert!(matches!(outcome, SwapOutcome::Skipped));
}

#[test]
#[serial]
fn swap_applied_writes_both_files_preserving_siblings() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // Rewrite live to look like account B — the swap should restore A.
    fs::write(
        home.live_credentials(),
        credentials_blob("uuid-B", 2_000).to_string(),
    )
    .unwrap();
    fs::write(
        home.live_config(),
        json!({
            "oauthAccount": { "accountUuid": "uuid-B", "emailAddress": "bob@x" },
            "projects": { "p1": { "keep": true } },
            "machineID": "machine-stays-put",
            "userID": "user-uuid-B",
            "hasAvailableSubscription": true,
        })
        .to_string(),
    )
    .unwrap();

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let outcome = swap_if_captured(&state, &provider).unwrap();
    assert!(matches!(outcome, SwapOutcome::Applied));

    let config: Value = serde_json::from_slice(&fs::read(home.live_config()).unwrap()).unwrap();
    assert_eq!(
        config
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-A")
    );
    // Machine state survives the swap.
    assert!(config.get("projects").is_some());
    assert_eq!(
        config.get("machineID").and_then(|v| v.as_str()),
        Some("machine-stays-put")
    );
    // Account state travels with the account, rather than being left behind.
    assert!(
        config.get("hasAvailableSubscription").is_none(),
        "B's entitlement must not linger once A is restored"
    );
    // userID belongs to this install, not to whichever account is signed in.
    assert_eq!(
        config.get("userID").and_then(|v| v.as_str()),
        Some("user-uuid-B")
    );
}

/// The regression that froze `/status`: a `.claude.json` inside the config
/// directory must not absorb the swap, leaving the file the CLI reads stale.
#[test]
#[serial]
fn swap_writes_the_config_the_cli_reads_not_a_decoy_inside_the_config_dir() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // A second, unrelated config sitting inside ~/.claude.
    let decoy = home.claude_dir().join(".claude.json");
    fs::write(
        &decoy,
        json!({ "oauthAccount": { "accountUuid": "uuid-DECOY", "emailAddress": "d@x" } })
            .to_string(),
    )
    .unwrap();
    // Live identity moves to B.
    fs::write(
        home.live_config(),
        json!({ "oauthAccount": { "accountUuid": "uuid-B", "emailAddress": "bob@x" } }).to_string(),
    )
    .unwrap();

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    swap_if_captured(&state, &provider).unwrap();

    let real: Value = serde_json::from_slice(&fs::read(home.live_config()).unwrap()).unwrap();
    assert_eq!(
        real.get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-A"),
        "the swap must land in the file Claude Code reads"
    );
    let untouched: Value = serde_json::from_slice(&fs::read(&decoy).unwrap()).unwrap();
    assert_eq!(
        untouched
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-DECOY"),
        "the decoy is someone else's config and stays untouched"
    );
}

#[test]
#[serial]
fn swap_aborts_when_snapshot_parse_fails() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // Corrupt one snapshot.
    fs::write(paths::snapshot_oauth_account_path("p1"), "not json").unwrap();

    // Capture current live state so we can verify it wasn't touched.
    let before_cred = fs::read(paths::live_credentials_path()).unwrap();
    let before_config = fs::read(paths::live_claude_config_path()).unwrap();

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let err = swap_if_captured(&state, &provider).unwrap_err();
    assert!(err.to_string().contains("corrupt"));

    assert_eq!(
        fs::read(paths::live_credentials_path()).unwrap(),
        before_cred
    );
    assert_eq!(
        fs::read(paths::live_claude_config_path()).unwrap(),
        before_config
    );
}

#[test]
#[serial]
fn swap_errors_when_snapshot_files_missing() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // Remove the snapshot dir behind Switchy's back but leave meta in place.
    fs::remove_dir_all(paths::snapshot_dir("p1")).unwrap();

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let err = swap_if_captured(&state, &provider).unwrap_err();
    assert!(err.to_string().contains("No captured snapshot"));
}

#[test]
#[serial]
fn swap_applied_with_mirror_when_mirror_configured_and_healthy() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // A mirror models the other install's ~/.claude dir; its .claude.json is
    // the sibling of that dir, exactly as on the local side.
    let mirror_home = home.path().join("wsl-home");
    let mirror = mirror_home.join(".claude");
    fs::create_dir_all(&mirror).unwrap();
    let mirror_config_path = mirror_home.join(".claude.json");
    fs::write(
        &mirror_config_path,
        json!({
            "oauthAccount": { "accountUuid": "uuid-OLD", "emailAddress": "old@x" },
            "statusLine": { "keep": true }
        })
        .to_string(),
    )
    .unwrap();
    set_mirror(&mirror);

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let outcome = swap_if_captured(&state, &provider).unwrap();
    assert!(matches!(outcome, SwapOutcome::AppliedWithMirror));

    let mirror_config: Value =
        serde_json::from_slice(&fs::read(&mirror_config_path).unwrap()).unwrap();
    assert_eq!(
        mirror_config
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-A")
    );
    // Sibling preserved.
    assert!(mirror_config.get("statusLine").is_some());
    // Mirror credentials written.
    assert!(mirror.join(".credentials.json").exists());
}

#[test]
#[serial]
fn swap_partial_mirror_when_mirror_config_unparseable() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    let mirror_home = home.path().join("wsl-home");
    let mirror = mirror_home.join(".claude");
    fs::create_dir_all(&mirror).unwrap();
    fs::write(mirror_home.join(".claude.json"), "not json").unwrap();
    set_mirror(&mirror);

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let outcome = swap_if_captured(&state, &provider).unwrap();
    match outcome {
        SwapOutcome::PartialMirror(warnings) => {
            assert!(warnings.iter().any(|w| w.ends_with(":parse")));
        }
        other => panic!("expected PartialMirror, got {other:?}"),
    }
    // Windows target still updated.
    let config: Value =
        serde_json::from_slice(&fs::read(paths::live_claude_config_path()).unwrap()).unwrap();
    assert_eq!(
        config
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-A")
    );
    // Mirror .credentials.json still written.
    assert!(mirror.join(".credentials.json").exists());
}

#[test]
#[serial]
fn swap_partial_mirror_when_mirror_unreachable() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();

    // Mirror dir that does not exist and we don't create.
    let unreachable = if cfg!(windows) {
        PathBuf::from("Z:/nonexistent-switchy-mirror")
    } else {
        PathBuf::from("/nonexistent-switchy-mirror")
    };
    set_mirror(&unreachable);

    let provider = state
        .db
        .get_provider_by_id("p1", "claude")
        .unwrap()
        .unwrap();
    let outcome = swap_if_captured(&state, &provider).unwrap();
    match outcome {
        SwapOutcome::PartialMirror(warnings) => {
            assert!(!warnings.is_empty());
            assert!(warnings.iter().all(|w| w.contains(":unreachable")
                || w.contains(":locked")
                || w.contains(":parse")));
        }
        other => panic!("expected PartialMirror, got {other:?}"),
    }
    // Windows side still applied.
    let config: Value =
        serde_json::from_slice(&fs::read(paths::live_claude_config_path()).unwrap()).unwrap();
    assert_eq!(
        config
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-A")
    );
}

#[test]
#[serial]
fn swap_syncs_outgoing_snapshot_before_overwriting_live() {
    // BACKLOG #5: capture A, let live creds "refresh" in the background,
    // then switch to B. A's snapshot must absorb the refreshed live creds
    // before B's snapshot clobbers the live file.
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("a"));
    seed_provider(&state, &official_provider("b"));

    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "a", false).unwrap();

    write_live_files(&home, "uuid-B", "bob@example.com");
    capture(&state, "b", false).unwrap();

    // Switch to A so Switchy records that A owns the live credentials.
    let provider_a = state.db.get_provider_by_id("a", "claude").unwrap().unwrap();
    swap_if_captured(&state, &provider_a).unwrap();

    // Simulate Claude Code background-refreshing A's tokens while A is live:
    // a rotated bundle with a later access-token expiry.
    fs::write(
        home.live_credentials(),
        credentials_blob("A-refreshed", 9_000).to_string(),
    )
    .unwrap();
    fs::write(
        home.live_config(),
        json!({
            "oauthAccount": {
                "accountUuid": "uuid-A",
                "emailAddress": "alice@example.com",
                "rotation": "new",
            },
            "projects": {}
        })
        .to_string(),
    )
    .unwrap();

    // Record A's snapshot before the switch.
    let before_cred =
        fs::read(paths::snapshot_credentials_path("a")).expect("A cred snapshot exists");

    // Switch to B — must first sync outgoing (A) from live, then apply B.
    let provider_b = state.db.get_provider_by_id("b", "claude").unwrap().unwrap();
    let outcome = swap_if_captured(&state, &provider_b).unwrap();
    assert!(matches!(outcome, SwapOutcome::Applied));

    // A's credential snapshot now reflects the refreshed live creds.
    let after_cred =
        fs::read(paths::snapshot_credentials_path("a")).expect("A cred snapshot still exists");
    assert_ne!(
        before_cred, after_cred,
        "A's snapshot should have been synced"
    );
    let after_val: Value = serde_json::from_slice(&after_cred).unwrap();
    assert_eq!(
        after_val
            .get("claudeAiOauth")
            .and_then(|v| v.get("refreshToken"))
            .and_then(|v| v.as_str()),
        Some("refresh-A-refreshed"),
        "the rotated refresh token is the only usable one and must be kept"
    );

    // A's oauthAccount snapshot updated too.
    let oauth_snap: Value =
        serde_json::from_slice(&fs::read(paths::snapshot_oauth_account_path("a")).unwrap())
            .unwrap();
    assert_eq!(
        oauth_snap.get("rotation").and_then(|v| v.as_str()),
        Some("new")
    );

    // Live now carries B's snapshot.
    let live_config: Value =
        serde_json::from_slice(&fs::read(paths::live_claude_config_path()).unwrap()).unwrap();
    assert_eq!(
        live_config
            .get("oauthAccount")
            .and_then(|v| v.get("accountUuid"))
            .and_then(|v| v.as_str()),
        Some("uuid-B")
    );
}

#[test]
#[serial]
fn swap_sync_noop_when_live_uuid_has_no_matching_captured_provider() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("b"));

    // Capture B with uuid-B.
    write_live_files(&home, "uuid-B", "bob@example.com");
    capture(&state, "b", false).unwrap();

    // Live is now some unknown account (fresh login, no Switchy capture).
    write_live_files(&home, "uuid-STRAY", "stray@example.com");

    let provider_b = state.db.get_provider_by_id("b", "claude").unwrap().unwrap();
    let outcome = swap_if_captured(&state, &provider_b).unwrap();
    assert!(matches!(outcome, SwapOutcome::Applied));
    // B's snapshot is unchanged (no stray match, no write).
    let oauth_snap: Value =
        serde_json::from_slice(&fs::read(paths::snapshot_oauth_account_path("b")).unwrap())
            .unwrap();
    assert_eq!(
        oauth_snap.get("accountUuid").and_then(|v| v.as_str()),
        Some("uuid-B")
    );
}

// ---------- switch-away sync: the three refusal rules ----------

/// Sets up two captured providers with A live and recorded as the owner.
fn seed_two_and_make_a_live(home: &ScopedHome, state: &AppState) {
    seed_provider(state, &official_provider("a"));
    seed_provider(state, &official_provider("b"));
    write_live_files(home, "uuid-A", "alice@example.com");
    capture(state, "a", false).unwrap();
    write_live_files(home, "uuid-B", "bob@example.com");
    capture(state, "b", false).unwrap();
    let provider_a = state.db.get_provider_by_id("a", "claude").unwrap().unwrap();
    swap_if_captured(state, &provider_a).unwrap();
}

fn switch_to_b(state: &AppState) {
    let provider_b = state.db.get_provider_by_id("b", "claude").unwrap().unwrap();
    swap_if_captured(state, &provider_b).unwrap();
}

/// A manual `claude /login` between switches replaces the live tokens with a
/// third account's. Filing those under A would destroy A's stored login.
#[test]
#[serial]
fn sync_out_refuses_when_live_login_is_not_the_recorded_owner() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_two_and_make_a_live(&home, &state);

    let before = fs::read(paths::snapshot_credentials_path("a")).unwrap();
    // Someone logs in as an entirely different account.
    write_live_files_at(&home, "uuid-STRANGER", "stranger@example.com", 9_000);

    switch_to_b(&state);

    assert_eq!(
        fs::read(paths::snapshot_credentials_path("a")).unwrap(),
        before,
        "A's snapshot must not absorb another account's tokens"
    );
}

/// A blanked bundle is what *losing* a refresh-token rotation leaves behind.
/// Storing it would replace a recoverable snapshot with a husk.
#[test]
#[serial]
fn sync_out_refuses_to_store_an_unusable_bundle() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_two_and_make_a_live(&home, &state);

    let before = fs::read(paths::snapshot_credentials_path("a")).unwrap();
    fs::write(
        home.live_credentials(),
        json!({
            "claudeAiOauth": {
                "accessToken": "access-A",
                "refreshToken": "",
                "expiresAt": 9_000,
            }
        })
        .to_string(),
    )
    .unwrap();

    switch_to_b(&state);

    assert_eq!(
        fs::read(paths::snapshot_credentials_path("a")).unwrap(),
        before,
        "a blanked refresh token must never overwrite a good snapshot"
    );
}

/// Freshness is one-directional: a live bundle older than the stored one is
/// stale state, not a refresh, and must not win.
#[test]
#[serial]
fn sync_out_refuses_to_overwrite_a_newer_snapshot_with_an_older_bundle() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_two_and_make_a_live(&home, &state);

    // Store a freshly refreshed bundle for A, then let live go backwards.
    fs::write(
        paths::snapshot_credentials_path("a"),
        credentials_blob("A-newest", 50_000).to_string(),
    )
    .unwrap();
    let before = fs::read(paths::snapshot_credentials_path("a")).unwrap();
    fs::write(
        home.live_credentials(),
        credentials_blob("A-older", 20_000).to_string(),
    )
    .unwrap();

    switch_to_b(&state);

    assert_eq!(
        fs::read(paths::snapshot_credentials_path("a")).unwrap(),
        before,
        "the older live bundle must not clobber the newer snapshot"
    );
}

/// The account-scoped keys beside `oauthAccount` are captured too, so a
/// restored account reports its own plan rather than the previous one's.
#[test]
#[serial]
fn capture_records_account_state_beside_the_identity() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    write_live_files(&home, "uuid-A", "alice@example.com");
    let mut root: Value = serde_json::from_slice(&fs::read(home.live_config()).unwrap()).unwrap();
    root["hasAvailableSubscription"] = json!(true);
    root["cachedUsageUtilization"] = json!({ "utilization": 42 });
    fs::write(home.live_config(), root.to_string()).unwrap();

    capture(&state, "p1", false).unwrap();

    let stored: Value =
        serde_json::from_slice(&fs::read(paths::snapshot_account_state_path("p1")).unwrap())
            .unwrap();
    assert_eq!(
        stored.get("hasAvailableSubscription").unwrap(),
        &json!(true)
    );
    assert_eq!(
        stored.get("cachedUsageUtilization").unwrap(),
        &json!({ "utilization": 42 })
    );
    assert!(
        stored.get("machineID").is_none(),
        "machine state is not part of the account"
    );
}

// ---------- read_captured_identity ----------

#[test]
#[serial]
fn read_captured_identity_returns_expected_states() {
    let home = ScopedHome::new();
    let state = make_state();
    seed_provider(&state, &official_provider("p1"));
    assert!(read_captured_identity(&state, "p1").unwrap().is_none());
    assert!(read_captured_identity(&state, "missing").unwrap().is_none());

    write_live_files(&home, "uuid-A", "alice@example.com");
    capture(&state, "p1", false).unwrap();
    let id = read_captured_identity(&state, "p1").unwrap().unwrap();
    assert_eq!(id.account_uuid, "uuid-A");

    clear(&state, "p1").unwrap();
    assert!(read_captured_identity(&state, "p1").unwrap().is_none());
}
