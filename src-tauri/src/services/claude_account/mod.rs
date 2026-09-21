//! Claude OAuth account snapshot + swap service.
//!
//! Per-provider capture of Claude Code's login identity, applied on switch
//! so switching Official providers also swaps `~/.claude/.credentials.json`
//! and the `oauthAccount` block in `.claude.json`.
//!
//! See `specs/official_multi_account/design.md` for the data-flow contract.

pub mod merge;
pub mod paths;
pub mod store;

#[cfg(target_os = "macos")]
mod keychain;

#[cfg(test)]
mod tests;

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::{CapturedClaudeAccountMeta, Provider, ProviderMeta};
use crate::store::AppState;

const CLAUDE_APP_TYPE: &str = "claude";
const OFFICIAL_CATEGORY: &str = "official";

/// Display-friendly identity returned to the renderer after a capture or
/// when hydrating the provider card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedIdentity {
    pub account_uuid: String,
    pub email_address: String,
    pub captured_at: i64,
}

impl From<CapturedClaudeAccountMeta> for CapturedIdentity {
    fn from(m: CapturedClaudeAccountMeta) -> Self {
        Self {
            account_uuid: m.account_uuid,
            email_address: m.email_address,
            captured_at: m.captured_at,
        }
    }
}

impl From<CapturedIdentity> for CapturedClaudeAccountMeta {
    fn from(i: CapturedIdentity) -> Self {
        Self {
            account_uuid: i.account_uuid,
            email_address: i.email_address,
            captured_at: i.captured_at,
        }
    }
}

/// Outcome of `capture`. `NeedsConfirmation` is returned when an existing
/// snapshot's `account_uuid` differs from the live login and the caller did
/// not pass `force=true` — the renderer then shows the AC-1.5 confirmation
/// dialog and re-invokes with `force=true` on Confirm.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CaptureOutcome {
    #[serde(rename_all = "camelCase")]
    Captured { identity: CapturedIdentity },
    #[serde(rename_all = "camelCase")]
    NeedsConfirmation {
        existing: CapturedIdentity,
        incoming: CapturedIdentity,
    },
}

#[derive(Debug)]
pub enum SwapOutcome {
    Skipped,
    Applied,
    AppliedWithMirror,
    PartialMirror(Vec<String>),
}

// =====================================================================
//  live credential store (platform-abstracted)
// =====================================================================

/// Reads Claude Code's live credentials blob from the authoritative store for
/// this platform: the macOS login Keychain (`Claude Code-credentials`), or the
/// `~/.claude/.credentials.json` file on Windows/Linux. Returns `Ok(None)`
/// when no login exists yet.
pub(crate) fn read_live_credentials() -> Result<Option<Vec<u8>>, AppError> {
    // The Keychain is a global side-channel the `SWITCHY_TEST_HOME` file
    // redirect can't sandbox, so tests bypass it and use the redirected file.
    #[cfg(target_os = "macos")]
    if std::env::var_os("SWITCHY_TEST_HOME").is_none() {
        if let Some(bytes) = keychain::read_credentials()? {
            return Ok(Some(bytes));
        }
        // Fall through to the file in case an older Claude Code wrote one.
    }

    let path = paths::live_credentials_path();
    match fs::read(&path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::io(&path, e)),
    }
}

/// Writes `blob` to Claude Code's live credential store for this platform: the
/// macOS Keychain, or `~/.claude/.credentials.json` on Windows/Linux.
pub(crate) fn write_live_credentials(blob: &[u8]) -> Result<(), AppError> {
    // See `read_live_credentials`: tests bypass the Keychain via the file path.
    #[cfg(target_os = "macos")]
    if std::env::var_os("SWITCHY_TEST_HOME").is_none() {
        return keychain::write_credentials(blob);
    }

    let target = paths::live_credentials_path();
    store::write_snapshot_atomic(&target, blob)
}

// =====================================================================
//  capture
// =====================================================================

/// Captures the live Claude Code identity for `provider_id` and persists it
/// under `~/.switchy/accounts/{provider_id}/`.
pub fn capture(
    state: &AppState,
    provider_id: &str,
    force: bool,
) -> Result<CaptureOutcome, AppError> {
    let provider = state
        .db
        .get_provider_by_id(provider_id, CLAUDE_APP_TYPE)?
        .ok_or_else(|| AppError::Message(format!("Provider {provider_id} not found")))?;

    // 1. Read live credentials blob from the platform's credential store
    //    (macOS Keychain, or ~/.claude/.credentials.json on Windows/Linux).
    let credentials_bytes = match read_live_credentials()? {
        Some(b) => b,
        None => {
            return Err(AppError::localized(
                "claudeAccount.capture.error.credentials_missing",
                "未找到 Claude Code 登录凭据，请先运行 `claude /login`",
                "No Claude Code login found. Run `claude /login` first.",
            ));
        }
    };
    // Parse-validate; we don't care about shape beyond "is JSON we can round-trip".
    let _: Value = serde_json::from_slice(&credentials_bytes).map_err(|_| {
        AppError::localized(
            "claudeAccount.capture.error.credentials_missing",
            "Claude Code 登录凭据文件损坏，请重新运行 `claude /login`",
            "Claude Code credentials file is unreadable. Re-run `claude /login`.",
        )
    })?;

    // 2. Read live Claude config and extract oauthAccount.
    let config_path = paths::live_claude_config_path();
    let live = read_oauth_from_live(&config_path)?;
    let LiveIdentity {
        root: live_root,
        oauth: oauth_account,
        account_uuid,
        email_address,
    } = live;

    // 3. Compare against existing snapshot UUID (AC-1.5).
    let existing_meta = provider
        .meta
        .as_ref()
        .and_then(|m| m.captured_claude_account.clone());
    if !force {
        if let Some(existing) = existing_meta.as_ref() {
            if existing.account_uuid != account_uuid {
                let incoming = CapturedIdentity {
                    account_uuid: account_uuid.clone(),
                    email_address: email_address.clone(),
                    captured_at: chrono::Utc::now().timestamp(),
                };
                return Ok(CaptureOutcome::NeedsConfirmation {
                    existing: existing.clone().into(),
                    incoming,
                });
            }
        }
    }

    // 4. Write snapshot files.
    store::write_snapshot_atomic(
        &paths::snapshot_credentials_path(provider_id),
        &credentials_bytes,
    )?;
    let oauth_bytes = serde_json::to_vec_pretty(&oauth_account)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(
        &paths::snapshot_oauth_account_path(provider_id),
        &oauth_bytes,
    )?;
    // The plan, entitlement and usage fields `/status` prints live beside
    // `oauthAccount`, not inside it; capture them together or the restored
    // account reads correctly on the identity line only.
    let account_state = merge::extract_account_state(&live_root);
    let state_bytes = serde_json::to_vec_pretty(&account_state)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(
        &paths::snapshot_account_state_path(provider_id),
        &state_bytes,
    )?;

    // 5. Update provider.meta and persist.
    let captured_at = chrono::Utc::now().timestamp();
    let identity = CapturedIdentity {
        account_uuid: account_uuid.clone(),
        email_address: email_address.clone(),
        captured_at,
    };
    let mut updated = provider.clone();
    let mut meta = updated.meta.take().unwrap_or_default();
    meta.captured_claude_account = Some(identity.clone().into());
    updated.meta = Some(meta);
    state.db.save_provider(CLAUDE_APP_TYPE, &updated)?;

    log::info!(
        "[claude_account] capture OK provider={} uuid={} email={}",
        provider_id,
        identity.account_uuid,
        identity.email_address
    );
    Ok(CaptureOutcome::Captured { identity })
}

/// The live login identity, plus the whole config root it came from so callers
/// can also lift the account-scoped sibling keys without re-reading the file.
struct LiveIdentity {
    root: Value,
    oauth: Value,
    account_uuid: String,
    email_address: String,
}

fn read_oauth_from_live(path: &Path) -> Result<LiveIdentity, AppError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(oauth_missing_err());
        }
        Err(e) => return Err(AppError::io(path, e)),
    };
    let root: Value = serde_json::from_slice(&bytes).map_err(|_| oauth_missing_err())?;
    let oauth = root
        .get("oauthAccount")
        .cloned()
        .filter(|v| v.is_object())
        .ok_or_else(oauth_missing_err)?;
    let uuid = oauth
        .get("accountUuid")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let email = oauth
        .get("emailAddress")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    if uuid.is_empty() || email.is_empty() {
        return Err(oauth_missing_err());
    }
    Ok(LiveIdentity {
        root,
        oauth,
        account_uuid: uuid,
        email_address: email,
    })
}

// =====================================================================
//  live owner marker
// =====================================================================

/// Record of whose credentials we last wrote into the live store.
///
/// The credentials blob carries no account identifier, so without this the
/// only way to attribute it on the next switch is to read the identity out of
/// `.claude.json` and trust that the two files agree. They can disagree — a
/// manual `claude /login`, or a config the app had been writing to the wrong
/// path — and a wrong guess files one account's tokens under another account's
/// snapshot, which destroys a working login. Recording the pairing at the
/// moment we create it removes the guess.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveOwner {
    #[serde(default)]
    version: u32,
    pub(crate) provider_id: String,
    pub(crate) account_uuid: String,
    /// `expiresAt` (ms) of the access token as written. A later value in the
    /// live file means Claude Code has refreshed since, and that refresh is
    /// what the switch-away sync exists to preserve.
    #[serde(default)]
    access_expires_at: i64,
}

const LIVE_OWNER_VERSION: u32 = 1;

pub(crate) fn read_live_owner() -> Option<LiveOwner> {
    let bytes = fs::read(paths::live_owner_path()).ok()?;
    serde_json::from_slice::<LiveOwner>(&bytes).ok()
}

pub(crate) fn write_live_owner(provider_id: &str, account_uuid: &str, access_expires_at: i64) {
    let owner = LiveOwner {
        version: LIVE_OWNER_VERSION,
        provider_id: provider_id.to_string(),
        account_uuid: account_uuid.to_string(),
        access_expires_at,
    };
    match serde_json::to_vec_pretty(&owner) {
        Ok(bytes) => {
            if let Err(e) = store::write_snapshot_atomic(&paths::live_owner_path(), &bytes) {
                log::warn!("[claude_account] live owner marker write failed: {e}");
            }
        }
        Err(e) => log::warn!("[claude_account] live owner marker serialize failed: {e}"),
    }
}

fn oauth_missing_err() -> AppError {
    AppError::localized(
        "claudeAccount.capture.error.oauth_missing",
        "Claude Code 配置缺少 oauthAccount，请先登录一次",
        "Claude Code config is missing the oauthAccount block. Open and use Claude Code once, then retry.",
    )
}

// =====================================================================
//  clear
// =====================================================================

/// Deletes the snapshot dir for `provider_id` and clears
/// `provider.meta.capturedClaudeAccount`. Idempotent; never touches
/// `~/.claude/` (AC-5.2).
pub fn clear(state: &AppState, provider_id: &str) -> Result<(), AppError> {
    log::info!("[claude_account] clear start provider={provider_id}");
    store::delete_snapshot_dir(provider_id)?;

    if let Some(mut provider) = state.db.get_provider_by_id(provider_id, CLAUDE_APP_TYPE)? {
        let needs_save = provider
            .meta
            .as_ref()
            .map(|m| m.captured_claude_account.is_some())
            .unwrap_or(false);
        if needs_save {
            if let Some(meta) = provider.meta.as_mut() {
                meta.captured_claude_account = None;
            }
            state.db.save_provider(CLAUDE_APP_TYPE, &provider)?;
            log::info!("[claude_account] clear wiped meta provider={provider_id}");
        } else {
            log::info!("[claude_account] clear no-op (no meta) provider={provider_id}");
        }
    }
    Ok(())
}

// =====================================================================
//  swap_if_captured
// =====================================================================

/// Restores the captured credentials + oauthAccount for `provider` when the
/// guard passes. Errors bubble up to the switch path, which converts them to
/// tagged warnings on `SwitchResult`.
pub fn swap_if_captured(state: &AppState, provider: &Provider) -> Result<SwapOutcome, AppError> {
    // Guard: Claude + Official + captured.
    if provider.category.as_deref() != Some(OFFICIAL_CATEGORY) {
        log::info!(
            "[claude_account] swap skip (not-official) provider={}",
            provider.id
        );
        return Ok(SwapOutcome::Skipped);
    }
    let Some(meta) = provider.meta.as_ref() else {
        log::info!(
            "[claude_account] swap skip (no meta) provider={}",
            provider.id
        );
        return Ok(SwapOutcome::Skipped);
    };
    if meta.captured_claude_account.is_none() {
        log::info!(
            "[claude_account] swap skip (no capture) provider={}",
            provider.id
        );
        return Ok(SwapOutcome::Skipped);
    }
    log::info!(
        "[claude_account] swap start provider={} uuid={}",
        provider.id,
        meta.captured_claude_account
            .as_ref()
            .map(|c| c.account_uuid.as_str())
            .unwrap_or("?")
    );

    // Switch-away sync (BACKLOG #5): before overwriting live creds with the
    // incoming snapshot, persist the current live creds back into whichever
    // captured provider they currently belong to. Claude Code refreshes tokens
    // in the background; without this sync those refreshes would be discarded
    // on each switch, collapsing the practical multi-account window.
    // Best-effort: any failure is logged but does not abort the switch.
    if let Err(e) = sync_outgoing_snapshot(state, &provider.id) {
        log::warn!(
            "[claude_account] sync_out error (non-fatal) for provider={}: {e}",
            provider.id
        );
    }

    let cred_snapshot = paths::snapshot_credentials_path(&provider.id);
    let oauth_snapshot = paths::snapshot_oauth_account_path(&provider.id);
    if !cred_snapshot.exists() || !oauth_snapshot.exists() {
        return Err(AppError::Message(format!(
            "No captured snapshot found for '{}'",
            provider.id
        )));
    }

    // Pre-flight parse gate (AC-2.4): both must parse before any write.
    let cred_value = store::read_snapshot(&cred_snapshot)?;
    let oauth_value = store::read_snapshot(&oauth_snapshot)?;
    // Snapshots captured before account_state.json existed simply don't have
    // one. Restore the identity alone rather than failing the switch; a
    // re-capture fills it in.
    let state_snapshot = paths::snapshot_account_state_path(&provider.id);
    let account_state = if state_snapshot.exists() {
        Some(store::read_snapshot(&state_snapshot)?)
    } else {
        log::info!(
            "[claude_account] no account_state snapshot for provider={} (captured by an older version); re-capture to carry plan and usage",
            provider.id
        );
        None
    };

    // --- Live credential store (Keychain on macOS, file on Windows/Linux) ---
    let cred_bytes = serde_json::to_vec_pretty(&cred_value)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    write_live_credentials(&cred_bytes)?;

    let target_config = paths::live_claude_config_path();
    let mut root = if target_config.exists() {
        let bytes = fs::read(&target_config).map_err(|e| AppError::io(&target_config, e))?;
        serde_json::from_slice::<Value>(&bytes).map_err(|e| AppError::json(&target_config, e))?
    } else {
        Value::Object(serde_json::Map::new())
    };
    merge::replace_oauth_account(&mut root, oauth_value.clone())?;
    if let Some(state) = account_state.as_ref() {
        merge::apply_account_state(&mut root, state)?;
    }
    let root_bytes =
        serde_json::to_vec_pretty(&root).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(&target_config, &root_bytes)?;

    // Record the pairing we just created, while it is still unambiguous.
    let (_, cred_expires_at) = crate::services::credential_mirror::credential_health(&cred_value);
    let swapped_uuid = oauth_value
        .get("accountUuid")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    write_live_owner(&provider.id, swapped_uuid, cred_expires_at);

    log::info!(
        "[claude_account] swap applied (live credential store) provider={}",
        provider.id
    );

    // --- Mirror targets (non-fatal) ---
    let Some(mirror_dir) = crate::settings::get_claude_mirror_override_dir() else {
        return Ok(SwapOutcome::Applied);
    };

    let mut warnings: Vec<String> = Vec::new();

    let mirror_cred = mirror_dir.join(".credentials.json");
    log::info!(
        "[claude_account] mirror cred write START path='{}' bytes={} provider={}",
        mirror_cred.display(),
        cred_bytes.len(),
        provider.id
    );
    match store::write_snapshot_atomic(&mirror_cred, &cred_bytes) {
        Ok(()) => {
            let mtime_ms = std::fs::metadata(&mirror_cred)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            log::info!(
                "[claude_account] mirror cred write OK path='{}' mtime_ms={} provider={}",
                mirror_cred.display(),
                mtime_ms,
                provider.id
            );
        }
        Err(e) => {
            log::warn!("credential mirror write failed for '{}': {e}", provider.id);
            let tag = classify_mirror_error(&e);
            warnings.push(format!("credential_mirror_failed:{}:{}", provider.id, tag));
        }
    }

    let mirror_config = paths::mirror_claude_config_path(&mirror_dir);
    match fs::read(&mirror_config) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(mut mirror_root) => {
                if merge::replace_oauth_account(&mut mirror_root, oauth_value).is_ok() {
                    if let Some(state) = account_state.as_ref() {
                        let _ = merge::apply_account_state(&mut mirror_root, state);
                    }
                    let out = serde_json::to_vec_pretty(&mirror_root)
                        .map_err(|e| AppError::JsonSerialize { source: e })?;
                    if let Err(e) = store::write_snapshot_atomic(&mirror_config, &out) {
                        log::warn!(
                            "mirror .claude.json write failed for '{}': {e}",
                            provider.id
                        );
                        let tag = classify_mirror_error(&e);
                        warnings.push(format!("credential_mirror_failed:{}:{}", provider.id, tag));
                    }
                }
            }
            Err(_) => {
                log::warn!("mirror .claude.json unparseable for '{}'", provider.id);
                warnings.push(format!("credential_mirror_failed:{}:parse", provider.id));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No mirror config yet — create one with the account block.
            let mut fresh = Value::Object(serde_json::Map::new());
            if merge::replace_oauth_account(&mut fresh, oauth_value).is_ok() {
                if let Some(state) = account_state.as_ref() {
                    let _ = merge::apply_account_state(&mut fresh, state);
                }
                let out = serde_json::to_vec_pretty(&fresh)
                    .map_err(|e| AppError::JsonSerialize { source: e })?;
                if let Err(e) = store::write_snapshot_atomic(&mirror_config, &out) {
                    log::warn!(
                        "mirror .claude.json create failed for '{}': {e}",
                        provider.id
                    );
                    let tag = classify_mirror_error(&e);
                    warnings.push(format!("credential_mirror_failed:{}:{}", provider.id, tag));
                }
            }
        }
        Err(e) => {
            log::warn!("mirror .claude.json read failed for '{}': {e}", provider.id);
            let tag = classify_mirror_error_from_io(&e);
            warnings.push(format!("credential_mirror_failed:{}:{}", provider.id, tag));
        }
    }

    if warnings.is_empty() {
        log::info!(
            "[claude_account] swap applied with mirror provider={}",
            provider.id
        );
        Ok(SwapOutcome::AppliedWithMirror)
    } else {
        log::info!(
            "[claude_account] swap partial mirror provider={} warnings={:?}",
            provider.id,
            warnings
        );
        Ok(SwapOutcome::PartialMirror(warnings))
    }
}

/// Files the outgoing account's live state back into its own snapshot before
/// the incoming account overwrites it. Claude Code rotates the refresh token
/// on every refresh (single-use, server-enforced), so a refresh that happened
/// while this account was live is the only usable copy — discard it and the
/// stored bundle is already dead.
///
/// Held to the same three rules as the Windows/WSL reconciler, for the same
/// reason: never act on a guess, never store a husk, never let an older bundle
/// overwrite a newer one. Best-effort throughout — anything uncertain is a
/// no-op, not an error, because failing here would block a switch the user
/// already asked for.
fn sync_outgoing_snapshot(state: &AppState, incoming_provider_id: &str) -> Result<(), AppError> {
    // 1. Attribution. Only the marker knows whose tokens are live.
    let Some(owner) = read_live_owner() else {
        log::info!(
            "[claude_account] sync_out skip: no live owner marker yet — the first switch records one"
        );
        return Ok(());
    };
    if owner.provider_id == incoming_provider_id {
        return Ok(());
    }
    let Some(outgoing) = state
        .db
        .get_provider_by_id(&owner.provider_id, CLAUDE_APP_TYPE)?
    else {
        log::info!(
            "[claude_account] sync_out skip: live owner provider={} no longer exists",
            owner.provider_id
        );
        return Ok(());
    };
    if outgoing
        .meta
        .as_ref()
        .and_then(|m| m.captured_claude_account.as_ref())
        .is_none()
    {
        log::info!(
            "[claude_account] sync_out skip: live owner provider={} is no longer captured",
            owner.provider_id
        );
        return Ok(());
    }

    // 2. The live login must still be the one we recorded. A manual
    //    `claude /login` in between replaces the tokens with a different
    //    account's, and filing those under this provider would destroy both.
    let config_path = paths::live_claude_config_path();
    let Ok(live) = read_oauth_from_live(&config_path) else {
        log::warn!("[claude_account] sync_out skip: live oauthAccount missing/unparseable");
        return Ok(());
    };
    if live.account_uuid != owner.account_uuid {
        log::warn!(
            "[claude_account] sync_out skip: live account {} does not match recorded owner {} — refusing to guess",
            live.account_uuid,
            owner.account_uuid
        );
        return Ok(());
    }

    // 3. Health. A blanked or unparseable bundle is what a *lost* refresh race
    //    leaves behind; storing it would overwrite a recoverable snapshot with
    //    a husk.
    let cred_bytes = match read_live_credentials() {
        Ok(Some(b)) => b,
        Ok(None) | Err(_) => return Ok(()),
    };
    let Ok(cred_value) = serde_json::from_slice::<Value>(&cred_bytes) else {
        log::warn!("[claude_account] sync_out skip: live creds unparseable");
        return Ok(());
    };
    let (alive, live_expires_at) =
        crate::services::credential_mirror::credential_health(&cred_value);
    if !alive {
        log::warn!(
            "[claude_account] sync_out skip: live bundle is not usable (blank access/refresh token)"
        );
        return Ok(());
    }

    // 4. Freshness. Later access-token expiry means more recently refreshed.
    let cred_snapshot = paths::snapshot_credentials_path(&owner.provider_id);
    if let Ok(existing) = store::read_snapshot(&cred_snapshot) {
        let (_, stored_expires_at) =
            crate::services::credential_mirror::credential_health(&existing);
        if stored_expires_at >= live_expires_at {
            log::info!(
                "[claude_account] sync_out skip: stored bundle for {} is not older (stored={} live={})",
                owner.provider_id,
                stored_expires_at,
                live_expires_at
            );
            return Ok(());
        }
    }

    if let Err(e) = store::write_snapshot_atomic(&cred_snapshot, &cred_bytes) {
        log::warn!(
            "[claude_account] sync_out credential write failed for {}: {e}",
            owner.provider_id
        );
        return Ok(());
    }
    let oauth_bytes = serde_json::to_vec_pretty(&live.oauth)
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    if let Err(e) = store::write_snapshot_atomic(
        &paths::snapshot_oauth_account_path(&owner.provider_id),
        &oauth_bytes,
    ) {
        log::warn!(
            "[claude_account] sync_out oauth write failed for {}: {e}",
            owner.provider_id
        );
        return Ok(());
    }
    let state_bytes = serde_json::to_vec_pretty(&merge::extract_account_state(&live.root))
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    if let Err(e) = store::write_snapshot_atomic(
        &paths::snapshot_account_state_path(&owner.provider_id),
        &state_bytes,
    ) {
        log::warn!(
            "[claude_account] sync_out account state write failed for {}: {e}",
            owner.provider_id
        );
    }

    log::info!(
        "[claude_account] sync_out updated outgoing provider={} uuid={} expiresAt={}",
        owner.provider_id,
        owner.account_uuid,
        live_expires_at
    );
    Ok(())
}

fn classify_mirror_error(err: &AppError) -> &'static str {
    match err {
        AppError::Io { source, .. } | AppError::IoContext { source, .. } => {
            classify_mirror_error_from_io(source)
        }
        _ => "unreachable",
    }
}

fn classify_mirror_error_from_io(e: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    match e.kind() {
        PermissionDenied => "locked",
        NotFound => "unreachable",
        _ => "unreachable",
    }
}

// =====================================================================
//  read_captured_identity
// =====================================================================

pub fn read_captured_identity(
    state: &AppState,
    provider_id: &str,
) -> Result<Option<CapturedIdentity>, AppError> {
    let Some(provider) = state.db.get_provider_by_id(provider_id, CLAUDE_APP_TYPE)? else {
        return Ok(None);
    };
    Ok(provider
        .meta
        .and_then(|m| m.captured_claude_account)
        .map(CapturedIdentity::from))
}

// Suppress unused-import lint when compiled without tests.
#[cfg(not(test))]
#[allow(dead_code)]
fn _used_types(_: &ProviderMeta, _: AppType) {}
