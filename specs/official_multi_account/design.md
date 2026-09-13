# Design — Official Multi-Account for Switchy

Based on `requirements.md` (same directory). Read requirements first.

## Overview

Switchy gains a per-Official-provider capture of Claude Code's OAuth identity. On switch to a captured Official provider, Switchy restores two files (credentials + oauthAccount merge) in addition to its existing `settings.json` write. Design goal: **reuse the existing switch transaction and WSL mirror enablement check**; add a parallel "credential swap pass" that either succeeds fully, fails cleanly with no partial writes, or is skipped when capture is absent.

**Upstream code this design extends:**
- `src-tauri/src/services/provider/live.rs:741-777` — `write_live_snapshot` Claude branch. New swap runs after this in the caller.
- `src-tauri/src/services/provider/mod.rs:1435-1548` — `ProviderService::switch_normal`. Injection point is between line 1509 (`write_live_with_common_config`) and line 1546 (`McpService::sync_all_enabled`).
- `src-tauri/src/settings.rs:543` — `get_claude_mirror_override_dir()` gates WSL mirror. Reused without change.
- `src-tauri/src/config.rs:21-33, 36-42, 89-122` — `get_home_dir`, `get_claude_config_dir`, `get_app_config_dir`. Reused without change.
- `src-tauri/src/provider.rs:225-301` — `ProviderMeta`. Extended with one new optional field.

**Downstream consumers unchanged except:**
- `SwitchResult.warnings: Vec<String>` (`mod.rs:46-50`) gains new warning tags (`credential_swap_failed:...`, `credential_mirror_failed:...`). Already string-typed, no shape change.
- `ProviderCard.tsx` renders a new optional identity subline when the provider has a captured account (new IPC read; see Component Design).

---

## Data Flow

### 1 — Capture flow

```
User clicks "Capture current account" on Official provider P
    │
    ▼
Renderer  ─ invoke "capture_claude_account" { providerId: P.id }
    │
    ▼
Rust command  claude_account::capture(state, provider_id)
    │
    ├─► Read  ~/.claude/.credentials.json        (fail if missing — AC-1.3)
    ├─► Read  ~/.claude/.claude.json  OR  ~/.claude.json   (see §Data Sources)
    ├─► Extract  .oauthAccount                    (fail if missing/malformed — AC-1.4)
    ├─► If provider already has snapshot AND new accountUuid differs from stored:
    │     return NeedsConfirmation { old_uuid, new_uuid, old_email, new_email }
    │     (renderer shows dialog; re-invokes with force=true on Confirm — AC-1.5)
    │
    ├─► Write  ~/.switchy/accounts/{P.id}/credentials.json  (atomic, 0600)
    ├─► Write  ~/.switchy/accounts/{P.id}/oauth_account.json
    └─► Update  Provider.meta.capturedClaudeAccount  in DB  (UUID + email + captured_at)
                                                                │
                                                                ▼
                                                        Renderer refetches
                                                        provider list → card
                                                        shows new identity
```

### 2 — Switch flow (Claude, Official category only)

```
ProviderService::switch_normal  (mod.rs:1435)
    │
    ├─ [existing] backfill current provider from live config          (unchanged)
    ├─ [existing] set_current_provider in settings + db                (unchanged)
    ├─ [existing] write_live_with_common_config                        (unchanged, line 1509)
    │   └─► writes ~/.claude/settings.json + WSL mirror (provider fields)
    │
    ├─ [NEW]  claude_account::swap_if_captured(state, provider)
    │   │
    │   ├─ Guard:  only runs when app_type == Claude
    │   │         AND provider.category == Some("official")
    │   │         AND provider.meta.capturedClaudeAccount.is_some()
    │   │         (otherwise returns Ok(SwapOutcome::Skipped))
    │   │
    │   ├─ Read both snapshot files under ~/.switchy/accounts/{id}/
    │   │   → parse-check only (AC-2.4). On parse failure: hard error.
    │   │
    │   ├─ Write  ~/.claude/.credentials.json  (atomic temp+rename, 0600)
    │   ├─ Merge  oauthAccount into Claude config file  (see §Data Sources Row 2 for target selection)
    │   │         — replace `.oauthAccount` key, leave siblings untouched
    │   │
    │   └─ If  get_claude_mirror_override_dir() == Some(dir):
    │         ├─ Write  {dir}/.credentials.json
    │         ├─ Merge  oauthAccount into {dir}/.claude.json  (with legacy-filename fallback per §Data Sources Row 3)
    │         └─ On per-file failure: log warn + append to result.warnings, continue
    │
    └─ [existing] McpService::sync_all_enabled                         (unchanged, line 1546)
```

### 3 — Clear flow

```
User clicks "Clear captured account"
    │
    ▼
Renderer  ─ invoke "clear_claude_account" { providerId }
    │
    ▼
Rust  delete snapshot files  +  clear Provider.meta.capturedClaudeAccount
    │
    (no ~/.claude/ writes — AC-5.2)
```

---

## Component Design

### C1 — New Rust module: `src-tauri/src/services/claude_account/`

One new module tree (not a submodule of `provider`) because credential handling is Claude-specific and does not generalize to Codex/Gemini. Placed alongside `mcp`, `omo`, `skill` under `services/`.

**Files:**

- `services/claude_account/mod.rs` — public API (`capture`, `clear`, `swap_if_captured`, `read_captured_identity`)
- `services/claude_account/paths.rs` — path helpers (snapshot dir, target file selection)
- `services/claude_account/store.rs` — snapshot file I/O (atomic write, read+parse-check)
- `services/claude_account/merge.rs` — `oauthAccount`-only merge helper
- `services/claude_account/tests.rs` — unit tests under `#[cfg(test)]`

**Public interface:**

```rust
pub struct CapturedIdentity {
    pub account_uuid: String,
    pub email_address: String,
    pub captured_at: i64,   // unix seconds
}

pub enum CaptureOutcome {
    Captured(CapturedIdentity),
    NeedsConfirmation {
        existing: CapturedIdentity,  // what's stored
        incoming: CapturedIdentity,  // what would replace it
    },
}

pub enum SwapOutcome {
    Skipped,           // not Claude, not Official, or no capture
    Applied,           // Windows target written, mirror skipped or not configured
    AppliedWithMirror, // Windows + WSL both written
    PartialMirror(Vec<String>), // Windows OK, mirror had N non-fatal failures — strings added to SwitchResult.warnings
}

pub fn capture(state: &AppState, provider_id: &str, force: bool) -> Result<CaptureOutcome, AppError>;
pub fn clear(state: &AppState, provider_id: &str) -> Result<(), AppError>;
pub fn swap_if_captured(state: &AppState, provider: &Provider) -> Result<SwapOutcome, AppError>;
pub fn read_captured_identity(state: &AppState, provider_id: &str) -> Result<Option<CapturedIdentity>, AppError>;
```

**Exact injection point.** `ProviderService::switch_normal` (`src-tauri/src/services/provider/mod.rs:1435-1548`) currently ends its Claude/Codex/Gemini path with, in order: `write_live_with_common_config` (line 1509) → `McpService::sync_all_enabled` (line 1546) → `Ok(result)` (line 1548). Insert the `swap_if_captured` call between those two existing calls — after line 1509, before line 1546. The insertion is inside the function body, not a wrapper:

```rust
write_live_with_common_config(state.db.as_ref(), &app_type, provider)?;

// NEW — AC-2.2: runs after settings write so a credential failure
// doesn't roll back the provider selection the user clicked.
match claude_account::swap_if_captured(state, provider) {
    Ok(SwapOutcome::PartialMirror(warnings)) => result.warnings.extend(warnings),
    Ok(_) => {}
    Err(e) => {
        // AC-2.2: surface as warning, not hard error — settings write already succeeded
        log::warn!("Claude account swap failed for '{}': {e}", provider.id);
        result.warnings.push(format!("credential_swap_failed:{}", provider.id));
    }
}

McpService::sync_all_enabled(state)?;
```

### C2 — New Tauri commands

File: `src-tauri/src/commands/claude_account.rs` (new)

```rust
#[tauri::command]
pub fn capture_claude_account(state: State<'_, AppState>, provider_id: String, force: bool)
    -> Result<CaptureOutcome, String>;

#[tauri::command]
pub fn clear_claude_account(state: State<'_, AppState>, provider_id: String)
    -> Result<(), String>;

// Read-only: used by the renderer to hydrate the card subline.
#[tauri::command]
pub fn get_captured_claude_identity(state: State<'_, AppState>, provider_id: String)
    -> Result<Option<CapturedIdentity>, String>;
```

All three register in `src-tauri/src/lib.rs` alongside other provider commands (follow the `commands::provider::*` registration pattern).

### C2.5 — Lifecycle hook: delete-provider integration

`ProviderService::delete_provider` (in `src-tauri/src/services/provider/mod.rs`; search `pub fn delete_provider` in the `ProviderService` impl) gains one call: invoke `claude_account::clear(state, id)` immediately before DB removal. If `clear` returns `Err`, log at `warn` and proceed with the DB delete — a stale snapshot directory is harmless (no provider row references it). This is load-bearing for AC-5.3 (deleting the provider must also delete its snapshot).

### C3 — `ProviderMeta` extension

File: `src-tauri/src/provider.rs:225-301` — add one field to the existing struct:

```rust
/// Captured Claude OAuth identity for this provider (Official/Claude only).
/// Presence implies a snapshot exists under ~/.switchy/accounts/{id}/.
#[serde(rename = "capturedClaudeAccount", skip_serializing_if = "Option::is_none")]
pub captured_claude_account: Option<CapturedClaudeAccountMeta>,
```

With:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedClaudeAccountMeta {
    pub account_uuid: String,
    pub email_address: String,
    pub captured_at: i64,
}
```

**Serialization contract (explicit to prevent TS/Rust drift):** `#[serde(rename_all = "camelCase")]` on the struct, combined with `#[serde(rename = "capturedClaudeAccount")]` on the field inside `ProviderMeta`, means the on-disk and over-IPC JSON shape is exactly:

```json
{ "capturedClaudeAccount": { "accountUuid": "...", "emailAddress": "...", "capturedAt": 1760000000 } }
```

The TS interface in `src/types.ts` (C4 below) must match this shape field-for-field. Any implementer adding a field here must add it in both places in the same edit.

**Why it lives in `ProviderMeta` and not a separate table:** `ProviderMeta` already uses the `~/.switchy/config.json` persistence path via the DB's provider row, and re-uses the `#[serde(skip_serializing_if)]` + camelCase convention the TS renderer already knows. Adding a sibling DB table would duplicate the "Provider + metadata" join for a single optional field.

**Why also store `email_address` in meta (duplicated with `oauth_account.json`):** the card needs to render the email without reading the snapshot on every refresh. The canonical source is still the snapshot file; meta is a denormalized cache updated on capture/clear only.

### C4 — Renderer changes

**File: `src/components/providers/ProviderCard.tsx`**

Add a subline below the provider name when the provider is Official-Claude AND has `meta.capturedClaudeAccount` set:

```tsx
{/* Renders when capturedClaudeAccount is present */}
<span className="text-xs text-muted-foreground" title={meta.capturedClaudeAccount.accountUuid}>
  {truncateEmail(meta.capturedClaudeAccount.emailAddress)}
</span>
```

`truncateEmail` is a new helper (`src/utils/truncateEmail.ts`) that keeps the first 12 chars of the local part + `@domain`. Exact length is a UI decision pinned here: **12 chars local-part cap**.

**File: `src/components/providers/forms/ProviderForm.tsx`** (Official-Claude path only)

Add two buttons in the edit dialog footer when appId === "claude" AND isOfficialProvider(…) is true:

- **Capture current account** — invokes `capture_claude_account`. On `NeedsConfirmation`, shows an AlertDialog: title "Account UUID changed", body shows stored vs. incoming email+UUID, Cancel / Confirm. Confirm re-invokes with `force=true`.
- **Clear captured account** — disabled when no snapshot. Invokes `clear_claude_account`.

No new components. Reuses existing `AlertDialog` from the design system.

**File: `src/types.ts:17`**

Extend the `ProviderMeta` TS interface with:

```ts
capturedClaudeAccount?: {
  accountUuid: string;
  emailAddress: string;
  capturedAt: number;
};
```

### C5 — i18n

New keys (namespace `claudeAccount`) added to `src/i18n/locales/{en,zh,ja}.json`:

- `claudeAccount.capture.button` — "Capture current account"
- `claudeAccount.capture.success` — "Captured {email}"
- `claudeAccount.capture.error.credentials_missing` — "No Claude Code login found. Run `claude /login` first."
- `claudeAccount.capture.error.oauth_missing` — "Claude Code config is missing the oauthAccount block. Open and use Claude Code once, then retry."
- `claudeAccount.capture.confirm.title` — "Account UUID changed"
- `claudeAccount.capture.confirm.body` — "Stored: {oldEmail}\nIncoming: {newEmail}\nOverwrite the stored snapshot?"
- `claudeAccount.clear.button` — "Clear captured account"
- `claudeAccount.clear.confirm` — "Forget this captured account? Claude Code stays logged in — this only removes Switchy's snapshot."
- `claudeAccount.swap.warning.locked` — "Swap failed: {path} is locked. Close Claude Code and try the switch again."
- `claudeAccount.swap.warning.mirror_skipped` — "WSL mirror skipped — {reason}"

Renderer resolves `credential_swap_failed:{id}` warnings from `SwitchResult.warnings` into the appropriate string.

---

## Data Sources

Every input, constant, and configuration value traced to its canonical source.

| # | Input | Source | Fallback / absence |
|---|---|---|---|
| 1 | Live credentials blob | `~/.claude/.credentials.json` via `get_claude_config_dir().join(".credentials.json")` (reuses existing `config::get_claude_config_dir`). | Missing at capture → hard error (AC-1.3). Missing at swap → not possible (capture already produced the snapshot). |
| 2 | Live Claude config (for capture and restore) | `config::get_claude_config_json_path()` — the **sibling** of the Claude config directory (`~/.claude` → `~/.claude.json`), never a file inside it. Resolved by rule, never by probing which candidate exists. `get_claude_mcp_path` delegates to the same function so no two writers can diverge. Missing at capture → hard error (AC-1.4). Missing at restore → create it. | See AC-2.1. |
| 3 | WSL mirror target for credentials | `get_claude_mirror_override_dir().join(".credentials.json")`. | If override is None: skip mirror entirely (AC-4.1 gate). |
| 4 | WSL mirror target for oauthAccount | `config::claude_config_json_for_dir(mirror_dir)` — the sibling of the mirror directory, matching Row 2: a mirror pointing at `\\wsl$\Ubuntu\home\me\.claude` writes `\\wsl$\Ubuntu\home\me\.claude.json`. **Note:** this is a *different file* from the existing provider-field mirror at `live.rs:748-755`, which chooses between `settings.json` and `claude.json` inside the mirror directory. The two operate on separate files. | Unparseable target: skip mirror oauthAccount update; still write `.credentials.json`. Log warn. (AC-4.3) |
| 5 | Captured credentials blob | `~/.switchy/accounts/{provider_id}/credentials.json` | At swap: parse failure → abort swap, no writes (AC-2.4). |
| 6 | Captured oauthAccount object | `~/.switchy/accounts/{provider_id}/oauth_account.json` | At swap: parse failure → abort swap, no writes (AC-2.4). |
| 7 | Captured identity for UI | `Provider.meta.capturedClaudeAccount` (DB-backed via existing provider storage). Denormalized; authoritative source is the `oauth_account.json` file. | Absent → card shows no identity (AC-3.2). |
| 8 | `account_uuid` for confirmation dialog | Compare `meta.capturedClaudeAccount.account_uuid` (stored) vs. newly read `.oauthAccount.accountUuid`. | See AC-1.5. |
| 9 | `captured_at` timestamp | `chrono::Utc::now().timestamp()` at capture time. | N/A — always set on capture. |
| 10 | Snapshot file permissions | Unix: `0o600` via `std::os::unix::fs::PermissionsExt`. Windows: rely on NTFS per-user ACL (default for files created under user profile). | Platform-dispatched; no user-visible config. |
| 11 | Home dir | `crate::config::get_home_dir()` (`config.rs:21`). Respects `SWITCHY_TEST_HOME`. | Falls back to `.` (existing behavior, unchanged). |

### `oauthAccount` observed shape (O-1 resolution)

Snapshot taken from `~/.claude.json` on user's Windows machine, **2026-04-16**:

| Field | Type | Notes |
|---|---|---|
| `accountUuid` | string | UUID. Switchy depends on this (identity key, AC-1.5). |
| `emailAddress` | string | Switchy depends on this (UI display, AC-1.2). |
| `organizationUuid` | string | Preserved opaque. |
| `organizationName` | string | Preserved opaque. |
| `organizationRole` | string (e.g. `"admin"`) | Preserved opaque. |
| `workspaceRole` | string or null | Preserved opaque. |
| `displayName` | string | Preserved opaque. |
| `hasExtraUsageEnabled` | bool | Preserved opaque. |
| `billingType` | string | Preserved opaque. |
| `accountCreatedAt` | string (ISO 8601) | Preserved opaque. |
| `subscriptionCreatedAt` | string (ISO 8601) | Preserved opaque. |

**Switchy depends on exactly two fields: `accountUuid` and `emailAddress`.** Every other field is preserved verbatim via whole-object replace. If Claude Code adds or renames other fields in the future, Switchy continues to work — only identity display would need updating if those two rename. This shape is for documentation; capture code validates only that `accountUuid` and `emailAddress` are present non-empty strings.

---

## Error Handling

| Condition | Detection point | Outcome | User-facing |
|---|---|---|---|
| `.credentials.json` missing at capture | `claude_account::capture`, before any writes | Return `AppError::localized("claudeAccount.capture.error.credentials_missing", …)` | Toast; AC-1.3 |
| `.claude.json`/`.claude.json` fallback both missing at capture | Same | `AppError::localized("claudeAccount.capture.error.oauth_missing", …)` | Toast; AC-1.4 |
| `oauthAccount` key missing / wrong type | Same | `AppError::localized("claudeAccount.capture.error.oauth_missing", …)` | Toast; AC-1.4 |
| `account_uuid`/`email_address` field missing or empty | Same | Same error key as above — user has no way to distinguish these cases, and the fix is the same (run Claude Code once). | Toast; AC-1.4 |
| New capture's UUID differs from stored | `capture` before overwriting | Return `CaptureOutcome::NeedsConfirmation` | AlertDialog; AC-1.5 |
| Snapshot file parse failure at swap | `claude_account::swap_if_captured` | Abort before any target write; return `AppError::Message("Captured snapshot for '{id}' is corrupt: {file}")`; caller converts to `credential_swap_failed:{id}` warning | Toast on switch result; AC-2.4 |
| Snapshot directory or files missing at swap (metadata present, files gone) | Same — probe existence before parse | Return `AppError::Message("No captured snapshot found for '{id}'")`. Should not happen in normal flow; indicates external interference with `~/.switchy/accounts/`. Caller converts to `credential_swap_failed:{id}` warning, same as parse failure. | Toast; AC-2.4 (same non-destructive outcome) |
| Windows target `.credentials.json` write fails (locked / EACCES) | Inside atomic write helper | Return error; caller records warning | AC-2.3 |
| Windows target `.claude.json` merge fails | Same | Same | AC-2.3 |
| Mirror directory unreachable | Probe failure on first mirror write | Log `warn`; push to `SwapOutcome::PartialMirror`; do **not** fail switch | AC-4.2 |
| Mirror `.claude.json` unparseable | Inside merge helper for mirror target | Skip oauthAccount merge on mirror; still write mirror `.credentials.json`; log warn | AC-4.3 |
| Atomic write: temp file created, rename fails | Inside atomic write helper | Clean up temp file; surface original error | Operational; not user-distinguished |
| Clear snapshot: files missing on disk | `claude_account::clear` | Idempotent — clear `meta.capturedClaudeAccount` regardless, treat "already gone" as success | AC-5.1 |

### Warning tag catalog

All tags that can appear in `SwitchResult.warnings` from this feature:

| Tag | Meaning | i18n key the renderer resolves to |
|---|---|---|
| `credential_swap_failed:{id}` | Windows credential/oauth write failed after settings write (covers: file locked, parse failure, missing snapshot dir) | `claudeAccount.swap.warning.failed_swap` (generic — message uses provider id) |
| `credential_mirror_failed:{id}:locked` | Mirror path locked by running Claude Code | `claudeAccount.swap.warning.locked` |
| `credential_mirror_failed:{id}:unreachable` | Mirror directory unreachable (WSL down, UNC path failed) | `claudeAccount.swap.warning.mirror_skipped` (reason = "unreachable") |
| `credential_mirror_failed:{id}:parse` | Mirror `.claude.json` unparseable | `claudeAccount.swap.warning.mirror_skipped` (reason = "parse") |

Tag strings are stable — the renderer's switch-on-prefix logic keys off them. Changes to the tag catalog require coordinated updates to `ProviderSwitchResultToast` (or wherever warnings are surfaced today; search `result.warnings` in `src/`).

**Hard failures vs. warnings, stated explicitly:**

- Capture: any failure is hard (no partial capture state written).
- Switch-time swap: Windows-target failure is a **warning** on `SwitchResult` (AC-2.2). Mirror-target failures are warnings (AC-4.2, 4.3). The only way to block a switch is an error from `write_live_with_common_config`, which is existing behavior and out of this spec's scope.

---

## Decision Inventory

### D-1 — Parallel swap pass vs. extending `write_live_snapshot`

**Decision:** Add a new `swap_if_captured` call alongside the existing Claude branch in `switch_normal`, rather than extending `write_live_snapshot` to handle credentials.

**Alternatives considered:**
- Extend `write_live_snapshot`'s Claude branch to also write `.credentials.json` and merge `oauthAccount`. Rejected: `write_live_snapshot` is called from multiple paths (`sync_current_to_live`, `sync_current_provider_for_app_to_live`, `sync_all_providers_to_live`), and **only** the switch path should apply an OAuth swap. Re-syncing on app startup must not silently overwrite the user's current Claude Code login. Keeping swap-on-switch-only in `switch_normal` is a cleaner boundary than threading "is this a real switch or a routine sync" into the shared path.

**Consequence:** The module has two independent code paths (settings write + credential swap) that share only the WSL mirror-dir lookup. This duplication is intentional.

### D-2 — `SwitchResult.warnings` for partial success vs. new typed return

**Decision:** Reuse the existing `SwitchResult { warnings: Vec<String> }` with new tagged warning strings (`credential_swap_failed:{id}`, `credential_mirror_failed:{id}:{reason}`).

**Alternatives considered:**
- Introduce a new `SwitchResult { settings_ok: bool, credential_ok: bool, warnings: Vec<String> }` as suggested in the requirements-stage review. Rejected: the existing `warnings` field is already how partial success is surfaced (backfill failures use the same pattern — `mod.rs:1491`), and changing the Tauri command return shape would ripple into every renderer consumer. Tagged warning strings are sufficient for the renderer to raise targeted toasts.

### D-3 — Per-provider snapshot dir keyed by `provider.id`

**Decision:** Snapshot files live at `~/.switchy/accounts/{provider.id}/{credentials.json, oauth_account.json}`.

**Alternatives considered:**
- Key by `account_uuid`. Rejected: deleting a provider row would orphan the snapshot. Keying by `provider.id` makes "delete provider → delete snapshot dir" a simple operation on one path.
- Flat file per provider (`~/.switchy/accounts/{id}.json` with both blobs embedded). Rejected: the credentials blob is opaque; bundling it with `oauthAccount` inside a wrapper JSON risks Switchy parsing what it should treat as bytes.

### D-4 — Restore-time merge: replace vs. deep-merge `oauthAccount`

**Decision:** Whole-object replace of `.oauthAccount`. Siblings (`projects`, other top-level keys) are untouched by merging a single key.

**Alternatives considered:**
- Field-by-field merge inside `oauthAccount`. Rejected: the user captured a coherent identity; merging fields across two accounts (e.g., A's email with B's UUID) produces a Frankenstein identity that Claude Code might accept but the user wouldn't understand.

### D-5 — Swap runs after settings write, not before

**Decision:** Settings write first, then credential swap.

**Rationale:** If the credential swap fails, the provider selection and env vars are already consistent with the user's click — the user is on "provider B" per Switchy, just still logged in as account A. Retrying the swap fixes the discrepancy. Reversed order would leave a user who failed the settings write logged in as B but with A's env vars, which is the harder state to recover from.

### D-6 — No automatic swap-on-startup

**Decision:** Swap runs only on explicit user switch. Startup/reload never touches `.credentials.json`.

**Rationale:** Resolves O-3. Silent startup overwrites would destroy manual edits the user may have made outside Switchy (e.g., re-running `claude /login` directly). Explicit action is the safer default.

### D-7 — Plaintext snapshot, `0600` on Unix

**Decision:** Snapshot files stored plaintext under `~/.switchy/accounts/{id}/` with `0o600` on Unix; default NTFS user-profile ACL on Windows. No encryption in v1.

**Rationale:** Claude Code's own `.credentials.json` is plaintext with the same permissions. Switchy's snapshot has the same sensitivity; introducing a key-management story (where does the encryption key live?) is a v2 problem. Documented in requirements "Out of scope".

### D-8 — 12-char local-part truncation for card display

**Decision:** `truncateEmail` keeps first 12 chars of local part + full `@domain`. Overflow → `...`.

**Rationale:** The observed emails during development (e.g. `user2@example.com`) are 10 chars local / long domain; 12 chars fits the common case without eliding the identity-bearing domain. If a user's email exceeds 12 local chars, the trailing `...` is visually distinct.

### D-9 — `ProviderMeta` field as denormalized cache of identity

**Decision:** `capturedClaudeAccount: Option<CapturedClaudeAccountMeta>` in `ProviderMeta` holds `account_uuid`, `email_address`, `captured_at` — duplicating data from `oauth_account.json`.

**Rationale:** UI hydration without per-card file I/O. The snapshot file remains authoritative for restore. Cache is invalidated atomically with the snapshot (same capture/clear transaction).

---

## Testing Strategy

**Rust unit tests** (added to `services/claude_account/tests.rs`, follow the existing `services::provider::live` test pattern):

- `capture_reads_credentials_and_oauthAccount_from_live_files` — uses `SWITCHY_TEST_HOME` to redirect `get_home_dir`.
- `capture_fails_when_credentials_missing` — asserts specific `AppError` key.
- `capture_fails_when_oauthAccount_missing` — asserts specific `AppError` key.
- `capture_returns_needs_confirmation_on_uuid_mismatch_unless_force_true`.
- `swap_if_captured_skipped_when_no_meta`.
- `swap_if_captured_writes_both_files_atomically_when_captured`.
- `swap_if_captured_aborts_before_any_write_on_snapshot_parse_failure`.
- `swap_replaces_oauthAccount_and_preserves_sibling_keys_in_claude_json`.
- `swap_mirror_skipped_when_override_dir_none`.
- `swap_mirror_partial_failure_returns_PartialMirror_and_does_not_fail_windows_write`.
- `clear_is_idempotent_when_files_absent`.

**Renderer typecheck**: `pnpm typecheck` passes after `types.ts` update.

**Renderer unit tests** (if time): new card subline renders when `capturedClaudeAccount` is present, hidden otherwise.

**Manual end-to-end** (captured in tasks stage, not run here):
1. Fresh Claude Code login as account A → capture on Default Official provider → card shows A's email.
2. Run `claude /login` as account B → add new Official provider → capture → card shows B's email.
3. Switch A → restart Claude Code → `claude /status` reports A. Switch B → `claude /status` reports B.
4. Configure WSL mirror → repeat step 3 → `cat ~/.claude/.claude.json | jq .oauthAccount.emailAddress` in WSL matches Windows.
5. Kill Claude Code mid-switch to trigger file-lock failure → switch returns warning, Switchy state still coherent.

---

## Backward Compatibility

- **Existing Official providers without capture:** `Provider.meta.capturedClaudeAccount` is `None`. `swap_if_captured` returns `Skipped`. Card renders exactly as today. Users can capture at their leisure (AC-3.2).
- **Existing non-Official providers:** Untouched. Guard in `swap_if_captured` rejects non-Claude and non-Official.
- **Deleting a provider with a snapshot:** See §C2.5 for the exact hook into `ProviderService::delete_provider`. (Moved out of this section because it is feature scope, not legacy compatibility.)
- **Upgrade from a Switchy version without this feature:** Old `config.json` lacks `capturedClaudeAccount` in `ProviderMeta` → deserialization defaults to `None` (field is `#[serde(skip_serializing_if = "Option::is_none")]`). No migration step needed.

---

## Platform notes

- **Windows atomic write:** `fs::write` to `{file}.tmp.{pid}` then `fs::rename` to target. Tokio's `spawn_blocking` not required — snapshot files are small (<20 KB), synchronous I/O on the main thread is acceptable given the existing `write_live_snapshot` pattern (`live.rs:741`) does the same.
- **Windows file locking:** If Claude Code holds `.credentials.json` open, `rename` returns `ERROR_SHARING_VIOLATION` → surfaces as `std::io::ErrorKind::PermissionDenied` in Rust. Wrapped into `AppError::io`. UI maps to the "locked" toast key. AC-2.3.
- **UNC / WSL path caveats:** `\\wsl$\Ubuntu-22.04\home\agentcode\.claude` is a regular UNC path. `std::fs::write` handles it. If WSL is not running, the write fails with `std::io::ErrorKind::NotFound` or timeout → mirror failure (AC-4.2), Windows-side swap still succeeds.
## Review dismissals

Iteration 1 review findings rebased per stage-locating rule:

- **"Expand §C4 renderer confirmation dialog routing (AlertDialog wiring, button handlers, i18n interpolation)"** — already specified in §C4: "On `NeedsConfirmation`, shows an AlertDialog... Cancel / Confirm. Confirm re-invokes with `force=true`." The i18n key structure (`claudeAccount.capture.confirm.{title,body}`) is in §C5 with `{oldEmail}` / `{newEmail}` placeholder names. Stage-3 (tasks) pins the exact renderer file path and function signature.

## Platform notes

- **No macOS code path in v1.** `#[cfg(target_os = "macos")]` stubs return `AppError::Message("macOS Keychain capture is not supported in this Switchy version")` rather than silently succeeding with wrong state. (Consistent with existing precedent at `src-tauri/src/services/provider/gemini_auth.rs` and `commit 602c5717`.)
