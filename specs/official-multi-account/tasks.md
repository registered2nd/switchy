# Tasks — Official Multi-Account for Switchy

Ordered, independently testable steps. Read `requirements.md` and `design.md` first. Each task is one coding session. Tick `- [ ]` → `- [x]` in the same edit pass as the code, per user-level CLAUDE.md "atomic close" rule.

Terminology: the `_Refs_` list names files to create or modify. `_Depends on:_` names preceding task IDs that must complete first. `Req:` cross-references the requirement + AC.

---

## Phase 1 — Rust data model & storage (no UI yet)

### T-1.1 Extend `ProviderMeta` with `capturedClaudeAccount`

Add the `CapturedClaudeAccountMeta` struct and the `captured_claude_account: Option<CapturedClaudeAccountMeta>` field to `ProviderMeta`, with `#[serde(rename_all = "camelCase")]` on the new struct and `#[serde(rename = "capturedClaudeAccount", skip_serializing_if = "Option::is_none")]` on the field. Fields: `account_uuid: String`, `email_address: String`, `captured_at: i64`.

**Verification:**
- `cargo test --lib` passes (no existing test should break).
- `cargo build` passes.
- Round-trip serialize a `ProviderMeta` with the new field populated and confirm the JSON contains the key `"capturedClaudeAccount"` with nested camelCase keys `accountUuid`, `emailAddress`, `capturedAt`. Put this in a new unit test inside the existing `#[cfg(test)] mod tests` block in `provider.rs` (or `provider/tests.rs` if one exists).
- Load an existing Switchy `~/.cc-switch/config.json` produced by the pre-feature build; ensure deserialization succeeds with `captured_claude_account: None`.

_Refs:_ `src-tauri/src/provider.rs:225-301`
_Req:_ supports US-1, US-2, US-5 — data model prerequisite
_Depends on:_ —

---

### T-1.2 Create `services/claude_account/` skeleton and paths helpers

Create the module tree under `src-tauri/src/services/claude_account/` with `mod.rs`, `paths.rs`, and an empty `tests.rs`. Wire `mod claude_account;` into `src-tauri/src/services/mod.rs`. Implement in `paths.rs`:

- `pub fn snapshot_dir(provider_id: &str) -> PathBuf` → `get_app_config_dir().join("accounts").join(provider_id)`
- `pub fn snapshot_credentials_path(provider_id: &str) -> PathBuf` → `snapshot_dir(id).join("credentials.json")`
- `pub fn snapshot_oauth_account_path(provider_id: &str) -> PathBuf` → `snapshot_dir(id).join("oauth_account.json")`
- `pub fn live_credentials_path() -> PathBuf` → `get_claude_config_dir().join(".credentials.json")`
- `pub fn live_claude_config_path() -> PathBuf` — selects per Design §Data Sources Row 2: primary `get_claude_config_dir().join(".claude.json")`, fallback `get_home_dir().join(".claude.json")`, else create primary.
- `pub fn mirror_claude_config_path(mirror_dir: &Path) -> PathBuf` — selects per Design §Data Sources Row 4: primary `mirror_dir.join(".claude.json")`, fallback `mirror_dir.join("claude.json")`, else primary.

**Verification:**
- Add unit tests using `CC_SWITCH_TEST_HOME` (`config.rs:22`) to redirect home dir into a tempdir, then assert each path function returns the expected `PathBuf` in each of the three cases (primary exists, only legacy exists, neither exists).
- `cargo test --lib services::claude_account::paths` passes with at least 4 tests covering the three cases for `live_claude_config_path` + a basic case for `snapshot_dir`.

_Refs:_ `src-tauri/src/services/claude_account/mod.rs` (new), `src-tauri/src/services/claude_account/paths.rs` (new), `src-tauri/src/services/claude_account/tests.rs` (new), `src-tauri/src/services/mod.rs`
_Req:_ US-1 AC-1.1, US-2 AC-2.1, US-4 AC-4.1 — path contracts
_Depends on:_ —

---

### T-1.3 Implement atomic snapshot store in `services/claude_account/store.rs`

Create `store.rs` with:

- `pub fn write_snapshot_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError>` — temp-file + rename, then `chmod 0o600` on Unix (`#[cfg(unix)]`). On Windows, rely on inherited ACL.
- `pub fn read_snapshot(path: &Path) -> Result<Value, AppError>` — reads and parses JSON, returning `AppError::Message("Captured snapshot for '{…}' is corrupt: {path}")` on parse failure (match the string format from Design §Error Handling).
- `pub fn delete_snapshot_dir(provider_id: &str) -> Result<(), AppError>` — removes the dir recursively; `Ok(())` if dir missing.

**Verification:**
- Unit test: `write_snapshot_atomic` followed by `read_snapshot` round-trips a JSON value.
- Unit test: `read_snapshot` on a file containing `not json` returns the exact error message shape from the design.
- Unit test: `delete_snapshot_dir` on a nonexistent provider id returns `Ok`.
- `cargo test --lib services::claude_account::store` passes.

_Refs:_ `src-tauri/src/services/claude_account/store.rs` (new)
_Req:_ US-1 AC-1.1, US-2 AC-2.4, US-5 AC-5.1
_Depends on:_ T-1.2

---

### T-1.4 Implement `oauthAccount`-only merge in `services/claude_account/merge.rs`

Create `merge.rs` with:

- `pub fn replace_oauth_account(target: &mut Value, oauth_account: Value) -> Result<(), AppError>` — expects `target` to be a JSON object; sets `target["oauthAccount"] = oauth_account`. Returns error if `target` is not an object.

This is the only merge primitive needed: whole-object replace of a single top-level key (Decision D-4). Do **not** port the existing `json_deep_merge` from `live.rs:173` — it has wrong semantics here (field-level merge would Frankenstein two accounts).

**Verification:**
- Unit test: replacing `oauthAccount` in an object with sibling keys (`projects`, `userID`, etc.) leaves siblings untouched.
- Unit test: calling with a non-object target returns the documented error.
- Unit test: calling on a target that has no prior `oauthAccount` key adds it.

_Refs:_ `src-tauri/src/services/claude_account/merge.rs` (new)
_Req:_ US-2 AC-2.1 (the merge semantics)
_Depends on:_ T-1.2

---

## Phase 2 — Capture / clear / swap operations

### T-2.1 Implement `capture` in `services/claude_account/mod.rs`

Public API per Design §C1:

```rust
pub fn capture(state: &AppState, provider_id: &str, force: bool) -> Result<CaptureOutcome, AppError>
```

Steps:
1. Look up the provider by id; bail if not found (`AppError::Message("Provider {id} not found")`).
2. Read `live_credentials_path()`. Missing → `AppError::localized("claudeAccount.capture.error.credentials_missing", zh: "未找到 Claude Code 登录凭据，请先运行 `claude /login`", en: "No Claude Code login found. Run `claude /login` first.")`.
3. Read `live_claude_config_path()`. If neither primary nor fallback exists → `AppError::localized("claudeAccount.capture.error.oauth_missing", zh: "Claude Code 配置缺少 oauthAccount，请先登录一次", en: "Claude Code config is missing the oauthAccount block. Open and use Claude Code once, then retry.")`. (Default strings sourced from Design §C5; keys are the same there.)
4. Extract `oauthAccount` object. Missing/non-object → same `oauth_missing` error. Missing or empty `accountUuid`/`emailAddress` → same error.
5. If `force == false` and `provider.meta.captured_claude_account.as_ref().map(|m| &m.account_uuid) != Some(&new_uuid)` **and** an existing capture is present: return `CaptureOutcome::NeedsConfirmation { existing, incoming }`.
6. Write snapshot files via `store::write_snapshot_atomic`.
7. Update `Provider.meta.captured_claude_account` and persist via `state.db.save_provider(…)`.
8. Return `CaptureOutcome::Captured(identity)`.

**Verification:**
- Unit test: happy path with both files present → `CaptureOutcome::Captured`, both snapshot files exist on disk, meta is set in DB.
- Unit test: `.credentials.json` absent → returns `credentials_missing` error key, no DB write, no file writes.
- Unit test: `.claude.json` present but missing `oauthAccount` → returns `oauth_missing` error key.
- Unit test: `oauthAccount` present but `accountUuid` is empty string → returns `oauth_missing` error key.
- Unit test: existing capture with matching UUID, force=false → silently overwrites (returns `Captured`, AC-1.5 silent case).
- Unit test: existing capture with *different* UUID, force=false → returns `NeedsConfirmation` with both identities populated, no file writes, no DB write.
- Unit test: same as above but force=true → overwrites (returns `Captured`).
- Use `CC_SWITCH_TEST_HOME` to isolate the filesystem. Use an in-memory or temp SQLite database via the existing `AppState` test helpers (see `src-tauri/src/services/provider/mod.rs:52+` for the existing pattern).

_Refs:_ `src-tauri/src/services/claude_account/mod.rs`
_Req:_ US-1 AC-1.1, AC-1.3, AC-1.4, AC-1.5
_Depends on:_ T-1.1, T-1.3, T-1.4

---

### T-2.2 Implement `clear` in `services/claude_account/mod.rs`

```rust
pub fn clear(state: &AppState, provider_id: &str) -> Result<(), AppError>
```

Steps:
1. Look up provider (ignore if not found — idempotent for delete ordering).
2. Call `store::delete_snapshot_dir(provider_id)` — treat missing dir as success.
3. If provider exists in DB, clear `meta.captured_claude_account` to `None` and persist. If no meta at all, that's fine.
4. Do **not** touch `~/.claude/` at all. AC-5.2.

**Verification:**
- Unit test: clear on a captured provider removes files and meta; second call is `Ok(())` (idempotent).
- Unit test: clear on a never-captured provider is `Ok(())`.
- Unit test: after clear, `get_claude_config_dir().join(".credentials.json")` is unchanged from its pre-clear state.

_Refs:_ `src-tauri/src/services/claude_account/mod.rs`
_Req:_ US-5 AC-5.1, AC-5.2
_Depends on:_ T-1.1, T-1.3

---

### T-2.3 Implement `swap_if_captured` in `services/claude_account/mod.rs`

```rust
pub fn swap_if_captured(state: &AppState, provider: &Provider) -> Result<SwapOutcome, AppError>
```

Steps per Design §Data Flow "2 — Switch flow":

1. Guard: return `Ok(SwapOutcome::Skipped)` unless all three hold: `app_type == Claude`, `provider.category.as_deref() == Some("official")`, `provider.meta.as_ref().and_then(|m| m.captured_claude_account.as_ref()).is_some()`.
2. **Pre-flight (no writes yet).** Probe existence of BOTH snapshot files; any missing → return `AppError::Message("No captured snapshot found for '{id}'")`. Then `read_snapshot` BOTH files; any parse failure → return `Err`. This is the AC-2.4 atomicity gate: the caller must not write any target file until both snapshots are verified readable.
3. **Windows targets (only reached after step 2 fully succeeds).**
   - Write `live_credentials_path()` via `store::write_snapshot_atomic` with the captured credentials bytes.
   - Read `live_claude_config_path()` (create-primary-if-missing). Apply `merge::replace_oauth_account`. Write back via `store::write_snapshot_atomic` with pretty-printed bytes.
   - If either Windows write fails (target file locked — AC-2.3), return `Err` immediately. If `.credentials.json` succeeded and `.claude.json` failed, the credentials file has already been rotated — this is acceptable per AC-2.2 (settings write already committed the provider selection, partial credential success is surfaced as a warning and the user retries).
6. If `settings::get_claude_mirror_override_dir()` returns `Some(dir)`:
   - Write `dir.join(".credentials.json")`. On failure, record `credential_mirror_failed:{id}:locked` or `:unreachable` per Design warning-tag catalog. Do not abort.
   - Read `mirror_claude_config_path(dir)`. If parse fails, record `credential_mirror_failed:{id}:parse` and skip the oauthAccount mirror step but continue.
   - If parse succeeds, `merge::replace_oauth_account` and write back.
7. Return `Applied`, `AppliedWithMirror`, or `PartialMirror(Vec<String>)` based on outcomes.

**Verification:**
Tests labelled by AC so the atomicity contracts don't get conflated:

- **AC-skip:** `Skipped` when provider is not Official.
- **AC-skip:** `Skipped` when captured meta is `None`.
- **AC-2.1:** `Applied` writes both `~/.claude/.credentials.json` and replaces `.oauthAccount` in the Claude config file; sibling keys in the config file preserved.
- **AC-2.4 (parse gate):** parse failure on one snapshot → returns `Err`; neither target file is modified. This verifies the *pre-flight gate* — both snapshots must be verified before the first write.
- **AC-2.4 (missing gate):** snapshot file missing on disk but meta says captured → returns `Err` with "No captured snapshot found" message; neither target file is modified.
- **AC-2.3 (target locked):** Windows `.credentials.json` write fails (simulate by making target read-only) → `Err`; `.claude.json` is **not** modified. Distinct from AC-2.4: here both snapshots were valid; the failure is on the target side. The credentials file was not rotated because write_atomic's temp-file + rename aborts cleanly on first-target lock.
- **AC-4.1 (mirror happy path):** mirror dir configured + mirror write succeeds → returns `AppliedWithMirror`; both Windows and mirror targets updated.
- **AC-4.3 (mirror parse):** mirror dir configured + mirror `.claude.json` unparseable → returns `PartialMirror(warnings)` with `credential_mirror_failed:{id}:parse`; Windows target still updated; mirror `.credentials.json` still written; mirror `oauthAccount` not updated.
- **AC-4.2 (mirror unreachable):** mirror dir set to unreachable path (`C:/nonexistent/path/.claude/`) → `PartialMirror` with `:unreachable`; Windows target still updated.

_Refs:_ `src-tauri/src/services/claude_account/mod.rs`, uses `src-tauri/src/settings.rs:543`
_Req:_ US-2 AC-2.1, AC-2.2, AC-2.3, AC-2.4; US-3 AC-3.1; US-4 AC-4.1, AC-4.2, AC-4.3, AC-4.4
_Depends on:_ T-1.1, T-1.3, T-1.4

---

### T-2.4 Implement `read_captured_identity` in `services/claude_account/mod.rs`

```rust
pub fn read_captured_identity(state: &AppState, provider_id: &str) -> Result<Option<CapturedIdentity>, AppError>
```

Simple read from `Provider.meta.captured_claude_account` (cached identity). Returns `None` if provider not found or no capture.

**Verification:**
- Unit test: returns `Some` after capture, `None` after clear, `None` for never-captured provider.

_Refs:_ `src-tauri/src/services/claude_account/mod.rs`
_Req:_ US-1 AC-1.2 (card hydration)
_Depends on:_ T-1.1

---

## Phase 3 — Wiring into existing flows

### T-3.1 Insert `swap_if_captured` into `switch_normal`

Modify `src-tauri/src/services/provider/mod.rs:1435-1548`. After the existing `write_live_with_common_config(...)?` call at line 1509 and **before** `McpService::sync_all_enabled(state)?;` at line 1546, insert exactly the block specified in Design §C1 "Exact injection point". The match handles `Ok(PartialMirror(w))`, other `Ok(_)`, and `Err(e)` — errors are logged and recorded in `result.warnings` as `credential_swap_failed:{id}`, not returned.

**Verification:**
- `cargo test --lib services::provider` passes (no regression on existing tests).
- New integration test in `services::provider::tests` (the `#[cfg(test)] mod tests` at line 52+): switch to an Official Claude provider with a captured snapshot → verify `.credentials.json` and `.claude.json` are updated and `SwitchResult.warnings` is empty.
- New integration test: switch with snapshot files deleted behind Switchy's back → `SwitchResult.warnings` contains `credential_swap_failed:{id}`; the settings write still succeeded.

_Refs:_ `src-tauri/src/services/provider/mod.rs:1509` (insertion point)
_Req:_ US-2 AC-2.2 (ordering + non-fatal partial success)
_Depends on:_ T-2.3

---

### T-3.2 Wire `claude_account::clear` into `delete_provider`

Search `fn delete_provider` in `src-tauri/src/services/provider/mod.rs`. Immediately before the DB removal, call `claude_account::clear(state, id)`. On `Err`, log `warn` and proceed with the DB delete (stale snapshot dir harmless).

**Verification:**
- Existing provider-delete tests still pass.
- New test: create an Official Claude provider, capture, delete the provider → assert `~/.cc-switch/accounts/{id}/` is gone AND the provider row is gone.
- New test: same as above but with `delete_snapshot_dir` stubbed to error → assert provider row is still deleted (warn logged, not returned).

_Refs:_ `src-tauri/src/services/provider/mod.rs` (search `fn delete_provider`)
_Req:_ US-5 AC-5.3
_Depends on:_ T-2.2

---

### T-3.3 Add Tauri command handlers in `commands/claude_account.rs`

Create `src-tauri/src/commands/claude_account.rs` with three `#[tauri::command]` functions matching Design §C2 signatures (`capture_claude_account`, `clear_claude_account`, `get_captured_claude_identity`). Register in `src-tauri/src/commands/mod.rs` and `src-tauri/src/lib.rs` following the existing `commands::provider::*` registration pattern.

**Verification:**
- `cargo build` succeeds.
- `pnpm tauri dev` launches without command-registration errors (manual check — the app starts).
- Invoke each command from the renderer's dev console via `invoke("capture_claude_account", { providerId: "default", force: false })` and verify the response shape matches design.

_Refs:_ `src-tauri/src/commands/claude_account.rs` (new), `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`
_Req:_ US-1, US-5 — renderer-facing IPC
_Depends on:_ T-2.1, T-2.2, T-2.4

---

## Phase 4 — Renderer

### T-4.1 Extend TS `ProviderMeta` type

Add `capturedClaudeAccount?: { accountUuid: string; emailAddress: string; capturedAt: number; }` to the appropriate interface in `src/types.ts`. Location: the interface used by `Provider.meta` (search for existing camelCase fields like `commonConfigEnabled`).

**Verification:**
- `pnpm typecheck` passes.

_Refs:_ `src/types.ts:17`
_Req:_ US-1 AC-1.2
_Depends on:_ T-1.1

---

### T-4.2 Create `src/utils/truncateEmail.ts`

Copy the implementation from Design §C4 verbatim. No deviations. Behavior summary: if input has an `@`, truncate local part to 12 chars (`…` suffix on overflow) and keep the full `@domain`; if input has no `@`, treat the whole string as local and truncate the same way; if input is empty, return empty string. All three branches must have tests.

**Verification:**
- New unit test file `src/utils/truncateEmail.test.ts`:
  - `truncateEmail("a@b.com")` → `"a@b.com"`
  - `truncateEmail("abcdefghijklmnop@domain.com")` → `"abcdefghijkl…@domain.com"` (12 chars + `…` + `@domain.com`)
  - `truncateEmail("noat")` → `"noat"`
  - `truncateEmail("reallylonglocalpartwithnoatsign")` → `"reallylonglo…"` (truncated without domain)
  - `truncateEmail("")` → `""`
- `pnpm test -- truncateEmail` passes.

_Refs:_ `src/utils/truncateEmail.ts` (new), `src/utils/truncateEmail.test.ts` (new)
_Req:_ US-1 AC-1.2 (display-only)
_Depends on:_ —

---

### T-4.3 Render captured identity subline in `ProviderCard`

Modify `src/components/providers/ProviderCard.tsx`. When `appId === "claude"` AND `isOfficialProvider(provider, appId)` AND `provider.meta?.capturedClaudeAccount` is set, render the subline from Design §C4 below the provider name. Use `truncateEmail` for the visible text and the full `accountUuid` as the `title` attribute for tooltip.

**Verification:**
- Visual check in `pnpm tauri dev`:
  - Official Claude provider with no capture → no subline (preserves current look).
  - After capture (done via dev console invoke for this task; UI capture comes in T-4.4) → subline appears with truncated email.
- `pnpm typecheck` passes.

_Refs:_ `src/components/providers/ProviderCard.tsx:60-81` (isOfficialProvider — reference location)
_Req:_ US-1 AC-1.2
_Depends on:_ T-3.3, T-4.1, T-4.2

---

### T-4.4 Add Capture/Clear buttons to `ProviderForm` (edit dialog)

Modify `src/components/providers/forms/ProviderForm.tsx`. When editing an Official Claude provider, render two buttons in the dialog footer:

- **Capture current account** — invokes `capture_claude_account` with `force: false`.
  - On success: toast `claudeAccount.capture.success` with the email.
  - On `NeedsConfirmation`: open AlertDialog with title `claudeAccount.capture.confirm.title`, body interpolated from `claudeAccount.capture.confirm.body` using `{oldEmail, newEmail}`; Confirm → re-invoke with `force: true`; Cancel → dismiss.
  - On `AppError` with localized key: toast that key.
- **Clear captured account** — disabled when `provider.meta?.capturedClaudeAccount` is absent. Invokes `clear_claude_account`. On confirmation dialog `claudeAccount.clear.confirm`.

Use existing `AlertDialog` primitive from the design system (search existing uses in `src/components/`).

**Verification:**
- Visual: open edit dialog for a Default Official provider → both buttons visible; Capture works end-to-end; Clear removes the subline on next card render.
- Visual: Capture against an already-captured provider with a new Claude Code login → confirmation dialog appears; Cancel leaves snapshot unchanged; Confirm overwrites.
- `pnpm typecheck` passes.
- `pnpm test` passes (if there are ProviderForm unit tests).

_Refs:_ `src/components/providers/forms/ProviderForm.tsx`
_Req:_ US-1 AC-1.1, AC-1.3, AC-1.4, AC-1.5; US-5 AC-5.1
_Depends on:_ T-3.3, T-4.1

---

### T-4.5 Add i18n keys

Add the keys from Design §C5 to `src/i18n/locales/en.json`, `zh.json`, and `ja.json`. English text from the design; zh/ja translations should be short and direct — keep English in a comment at the end of each line if translation fidelity is uncertain, for later review.

**Verification:**
- `pnpm typecheck` passes (i18n typed-keys plugin, if any).
- Launch each locale via the existing language switcher → spot-check each new key renders, not as raw `claudeAccount.*`.

_Refs:_ `src/i18n/locales/en.json`, `src/i18n/locales/zh.json`, `src/i18n/locales/ja.json`
_Req:_ US-1 error messages; US-2 warning toasts; US-5 confirm dialog
_Depends on:_ T-4.4

---

### T-4.6 Surface `credential_swap_failed:*` / `credential_mirror_failed:*` warnings in switch toast

Search the renderer for the current consumer of `SwitchResult.warnings`. Extend it to parse the tag catalog from Design §Error Handling and map each prefix to the corresponding `claudeAccount.swap.warning.*` i18n key. Tag grammar:

- `credential_swap_failed:{id}` → generic `failed_swap` key
- `credential_mirror_failed:{id}:{reason}` → reason-specific key (`locked`, `mirror_skipped`)

**Verification:**
- Integration test (manual): capture two Official providers, open one for editing inside Claude Code so `.credentials.json` is locked, switch → toast shows localized "locked" warning.
- Manual test: configure a bogus mirror dir (e.g. `C:/nonexistent/.claude`), switch → toast shows "unreachable"-variant warning.

_Refs:_ search `result.warnings` or `SwitchResult` usage in `src/`
_Req:_ US-2 AC-2.3, US-4 AC-4.2, AC-4.3
_Depends on:_ T-4.5

---

## Phase 5 — End-to-end validation

### T-5.1 Manual E2E walkthrough on Windows only (no mirror)

Run through Requirements §Success Criteria items 1–3 on the user's actual machine. For each step, note observed vs. expected.

**Verification checklist:**
1. [ ] Logged in as account A in Claude Code → open Switchy, pick any existing Official Claude provider row (either the imported default entry or one manually created with empty `ANTHROPIC_BASE_URL`) → run Capture → card subline shows A's email.
2. [ ] `claude /login` to account B → add new Official provider in Switchy UI → capture → subline shows B's email.
3. [ ] Switch to A → open a new Claude Code session → `claude /status` (or equivalent) reports A.
4. [ ] Switch back to B → new session reports B.
5. [ ] During step 4, if Claude Code was open: reproduce the locked-file warning; toast appears; switch is non-destructive; retry after closing Claude Code succeeds.

_Refs:_ —
_Req:_ Success criteria 1–3
_Depends on:_ T-4.6

---

### T-5.2 Manual E2E with WSL mirror configured

Same as T-5.1 but with `Claude Code Mirror Directory` set to `\\wsl$\Ubuntu-22.04\home\agentcode\.claude` (or the user's actual WSL Claude dir). Validate Requirements §Success Criteria item 4.

**Verification checklist:**
1. [ ] Switch to A on Windows → inside WSL, `cat ~/.claude/.claude.json | jq .oauthAccount.emailAddress` returns A's email.
2. [ ] Inside WSL, `cat ~/.claude/.credentials.json | jq type` returns `"object"` (i.e. file was replaced, not corrupted).
3. [ ] Switch to B → same two commands reflect B.
4. [ ] Verify existing WSL hooks / statusLine in `~/.claude/settings.json` were NOT modified (AC-4.4 regression guard). Use `git diff` if the WSL `.claude` is under version control, else spot-check keys.
5. [ ] Shut WSL down (`wsl --shutdown`), switch in Switchy → toast shows `unreachable` warning; Windows state still updated.

_Refs:_ —
_Req:_ Success criterion 4; US-4 regressions
_Depends on:_ T-5.1

---

### T-5.3 Build Windows installer with feature enabled

Run the existing build pipeline (`pnpm tauri build`) from a clean tree. Confirm the installer builds and version is bumped appropriately (follow the pattern in `src-tauri/tauri.conf.json` — bump patch version). Installer ends up at `src-tauri/target/release/bundle/nsis/Switchy_{version}_x64-setup.exe`.

**Verification:**
- Installer exists at expected path.
- Installing it over the existing Switchy preserves `~/.cc-switch/` data (no data loss — regression guard for `DECISION_LOG.md` 2026-04-07 namespace continuity decision).
- Fresh install on a clean profile opens without error.

_Refs:_ `src-tauri/tauri.conf.json` (version bump)
_Req:_ ship-ready criterion
_Depends on:_ T-5.1, T-5.2

---

## Accepted tradeoffs

None carried forward from requirements or design review loops — all raised P1s addressed in-stage.

## Post-merge follow-ups (not this spec)

- Fork Polish Punch List (BACKLOG.md) remains unblocked and ready for a later session.
- macOS Keychain support — deferred per Requirements "Out of scope". Separate spec when needed.
- Encryption of snapshot store — deferred per Requirements "Out of scope".
