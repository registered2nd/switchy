//! Claude OAuth account snapshot + swap service.
//!
//! Per-provider capture of Claude Code's login identity, applied on switch
//! so switching Official providers also swaps `~/.claude/.credentials.json`
//! and the `oauthAccount` block in `.claude.json`.
//!
//! See `specs/official-multi-account/design.md` for the data-flow contract.

pub mod merge;
pub mod paths;
pub mod store;

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

    // 1. Read live credentials blob.
    let credentials_path = paths::live_credentials_path();
    let credentials_bytes = match fs::read(&credentials_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::localized(
                "claudeAccount.capture.error.credentials_missing",
                "未找到 Claude Code 登录凭据，请先运行 `claude /login`",
                "No Claude Code login found. Run `claude /login` first.",
            ));
        }
        Err(e) => return Err(AppError::io(&credentials_path, e)),
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
    let (oauth_account, account_uuid, email_address) = read_oauth_from_live(&config_path)?;

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
    let oauth_bytes =
        serde_json::to_vec_pretty(&oauth_account).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(
        &paths::snapshot_oauth_account_path(provider_id),
        &oauth_bytes,
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

fn read_oauth_from_live(path: &Path) -> Result<(Value, String, String), AppError> {
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
    Ok((oauth, uuid, email))
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
pub fn swap_if_captured(
    _state: &AppState,
    provider: &Provider,
) -> Result<SwapOutcome, AppError> {
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

    // --- Windows targets ---
    let cred_bytes =
        serde_json::to_vec_pretty(&cred_value).map_err(|e| AppError::JsonSerialize { source: e })?;
    let target_cred = paths::live_credentials_path();
    store::write_snapshot_atomic(&target_cred, &cred_bytes)?;

    let target_config = paths::live_claude_config_path();
    let mut root = if target_config.exists() {
        let bytes = fs::read(&target_config).map_err(|e| AppError::io(&target_config, e))?;
        serde_json::from_slice::<Value>(&bytes).map_err(|e| AppError::json(&target_config, e))?
    } else {
        Value::Object(serde_json::Map::new())
    };
    merge::replace_oauth_account(&mut root, oauth_value.clone())?;
    let root_bytes =
        serde_json::to_vec_pretty(&root).map_err(|e| AppError::JsonSerialize { source: e })?;
    store::write_snapshot_atomic(&target_config, &root_bytes)?;

    log::info!(
        "[claude_account] swap applied (Windows targets) provider={}",
        provider.id
    );

    // --- Mirror targets (non-fatal) ---
    let Some(mirror_dir) = crate::settings::get_claude_mirror_override_dir() else {
        return Ok(SwapOutcome::Applied);
    };

    let mut warnings: Vec<String> = Vec::new();

    let mirror_cred = mirror_dir.join(".credentials.json");
    if let Err(e) = store::write_snapshot_atomic(&mirror_cred, &cred_bytes) {
        log::warn!(
            "credential mirror write failed for '{}': {e}",
            provider.id
        );
        let tag = classify_mirror_error(&e);
        warnings.push(format!("credential_mirror_failed:{}:{}", provider.id, tag));
    }

    let mirror_config = paths::mirror_claude_config_path(&mirror_dir);
    match fs::read(&mirror_config) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(mut mirror_root) => {
                if merge::replace_oauth_account(&mut mirror_root, oauth_value).is_ok() {
                    let out = serde_json::to_vec_pretty(&mirror_root)
                        .map_err(|e| AppError::JsonSerialize { source: e })?;
                    if let Err(e) = store::write_snapshot_atomic(&mirror_config, &out) {
                        log::warn!(
                            "mirror .claude.json write failed for '{}': {e}",
                            provider.id
                        );
                        let tag = classify_mirror_error(&e);
                        warnings.push(format!(
                            "credential_mirror_failed:{}:{}",
                            provider.id, tag
                        ));
                    }
                }
            }
            Err(_) => {
                log::warn!("mirror .claude.json unparseable for '{}'", provider.id);
                warnings.push(format!(
                    "credential_mirror_failed:{}:parse",
                    provider.id
                ));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No mirror config yet — create one with just oauthAccount.
            let mut fresh = Value::Object(serde_json::Map::new());
            if merge::replace_oauth_account(&mut fresh, oauth_value).is_ok() {
                let out = serde_json::to_vec_pretty(&fresh)
                    .map_err(|e| AppError::JsonSerialize { source: e })?;
                if let Err(e) = store::write_snapshot_atomic(&mirror_config, &out) {
                    log::warn!(
                        "mirror .claude.json create failed for '{}': {e}",
                        provider.id
                    );
                    let tag = classify_mirror_error(&e);
                    warnings.push(format!(
                        "credential_mirror_failed:{}:{}",
                        provider.id, tag
                    ));
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
