# Requirements — Official Multi-Account for Switchy

## Purpose

Let a Switchy user manage two or more distinct Anthropic accounts under the existing "Official" provider category. Switching between two Official providers must swap Claude Code's OAuth identity, not just env vars, so each provider represents a real separate Anthropic account (different subscription tier, different billing, different login).

Today the "Official" category is defined by the *absence* of `ANTHROPIC_BASE_URL` / `ANTHROPIC_API_KEY` (`src/components/providers/ProviderCard.tsx:60-80` `isOfficialProvider`). Switchy never touches Claude Code's OAuth state, so two "Official" rows currently collapse to whichever account Claude Code last logged in as. This spec owns the fix.

## Scope boundary

This spec owns:

- Capture of Claude Code OAuth state per Official provider.
- Restore of OAuth state when switching to that Official provider.
- UI identity indicator on the Official provider card so two entries are distinguishable.
- WSL mirror of the credential swap (symmetric with the existing provider-field mirror).
- Migration for existing Official entries that predate this feature.

This spec does NOT own:

- Non-Official provider behavior (third-party providers keep today's env-only semantics — unchanged).
- The existing WSL provider-field mirror semantics (`env` subset + `model` + `permissions` + `effortLevel`) — see `docs/fork_notes_windows_wsl_claude_sync.md` and `DECISION_LOG.md` 2026-04-07. This spec adds a parallel credential-swap pass; it does not revisit the provider-field decision.
- macOS Keychain handling (deferred — see "Out of scope").
- Codex / Gemini / OpenCode / OpenClaw apps (Claude only in v1).
- The existing `~/.switchy/` namespace (see `DECISION_LOG.md` 2026-04-07).

## Upstream dependencies

External state Switchy reads to capture an account:

| Source | Purpose | Shape / fields Switchy depends on |
|---|---|---|
| `~/.claude/.credentials.json` (Windows + WSL file) | Claude Code OAuth tokens | Opaque JSON blob. Switchy reads whole-file and writes whole-file. Does not parse fields. |
| `~/.claude/.claude.json` **or** `~/.claude.json` (fallback) | Claude Code full config; contains the OAuth account identity | Depends on one field: `oauthAccount` (object). Switchy reads its `emailAddress` (string) and `accountUuid` (string) for UI display, and the whole `oauthAccount` object for restore-time merge. **Field-shape assumption to be confirmed during design by inspecting a live file — see "Open question O-1".** |
| `~/.claude/settings.json` (primary) / `~/.claude/claude.json` (legacy) | Existing provider env state | Already read by `write_live_snapshot` in `src-tauri/src/services/provider/live.rs:743-777`. Not touched by this spec except for ordering. |

External state the WSL mirror target exposes:

| Source | Purpose |
|---|---|
| Mirror directory resolved by `get_claude_mirror_override_dir()` in `src-tauri/src/settings.rs:543` | Optional WSL `.claude/` folder. May or may not exist. May or may not be reachable (UNC path). |

## Downstream consumers

- Claude Code CLI reads `~/.claude/.credentials.json` and `~/.claude/.claude.json` on launch. It is the only consumer of the restored state. Switchy must produce files Claude Code accepts.
- Switchy's existing provider switch command (`ProviderService::switch` — entry in `src-tauri/src/commands/provider.rs:83-88`) calls `sync_current_to_live` (`src-tauri/src/services/provider/live.rs:968`). This spec adds work that must run inside the same switch transaction so the UI does not see a partial swap.
- The existing WSL mirror writer in `live.rs:748-777` runs on every Claude live write. This spec's WSL credential mirror must follow the same enablement check (`get_claude_mirror_override_dir` returns `Some`) so users do not need a second toggle.

## Ownership: where OAuth state lives

| Concern | Owner |
|---|---|
| OAuth token file format | Claude Code — Switchy treats as opaque. |
| `oauthAccount` JSON shape | Claude Code — Switchy depends only on `emailAddress` + `accountUuid` for display and preserves the rest on restore. |
| Per-provider captured snapshot storage | Switchy — lives under its existing `~/.switchy/` data dir (see `get_app_config_dir` in `src-tauri/src/config.rs:89-122`). Exact sub-layout is a design decision, not a requirements decision. |

## User stories

### US-1 — Add a second Official account

**As a** Switchy user already logged into Claude Code with Anthropic account A, **I want to** add a second Official provider that represents Anthropic account B, **so that** I can switch between A and B without re-running `claude /login` each time.

**Acceptance criteria**

- **AC-1.1** Switchy exposes a capture action on an Official Claude provider (add or edit dialog, or card). Invoking it reads the current state at `~/.claude/.credentials.json` and the `oauthAccount` block from `~/.claude/.claude.json` (or `~/.claude.json` fallback per the reference impl at https://github.com/ming86/cc-account-switcher) and stores it in Switchy's per-provider snapshot location.
- **AC-1.2** After capture, that provider's card displays the captured account's `emailAddress` (truncated if long) and a badge indicating the provider has a captured identity. A second Official provider without a capture shows no identity — only the default "Official" label.
- **AC-1.3** If `~/.claude/.credentials.json` is missing at capture time, the capture action fails with a localized error explaining the user must be logged into Claude Code first. No partial snapshot is written.
- **AC-1.4** If the `oauthAccount` block is missing or malformed, the capture action fails with a localized error. No partial snapshot is written. (Rationale: without `oauthAccount`, Switchy has no way to render an identity, and silently capturing just the credentials file would produce an un-distinguishable second entry.)
- **AC-1.5** Capture is re-runnable — re-capturing on the same provider overwrites that provider's snapshot. If the newly captured `accountUuid` matches the one already stored, overwrite is silent. If it differs, Switchy shows a confirmation dialog ("Account UUID has changed since last capture. Overwrite?") — Confirm overwrites the snapshot, Cancel aborts with no change to the stored snapshot. (Rationale: a UUID mismatch means the user logged into a different account in Claude Code since the last capture — more likely a mistake than a rename, so make them affirm.)

### US-2 — Switch between two captured Official accounts

**As a** user with Official providers A and B both captured, **I want to** click "switch" on B and have Claude Code start using account B's identity on my next run, **so that** subscription and billing are correctly attributed.

**Acceptance criteria**

- **AC-2.1** On switch to an Official provider with a captured snapshot, Switchy restores both files: it overwrites `~/.claude/.credentials.json` with the snapshot's credentials blob, and merges the snapshot's `oauthAccount` object into the Claude Code config file, replacing any existing `oauthAccount` while leaving sibling fields (`projects`, any user settings) untouched. The target config file is selected deterministically: primary = `~/.claude/.claude.json`; if that does not exist, fall back to `~/.claude.json`; if neither exists, create the primary path. (This mirrors the ming86 reference impl's `get_claude_config_path` behavior.)
- **AC-2.2** The credential swap runs **after** the existing `write_live_snapshot` Claude settings write (`live.rs:741-777`) so that if the credential swap fails, the settings write has already succeeded and the provider selection remains consistent with what the user clicked. The swap result is surfaced in the switch command's return value so the UI can raise a toast on partial success.
- **AC-2.3** If Claude Code is running during the switch and the file is locked, the swap fails with a localized error naming the file that was locked. The previous credential state is left in place (no half-written file). The failure is non-destructive — the user can retry after closing Claude Code.
- **AC-2.4** Before writing, Switchy runs a minimal JSON-parse check on each snapshot file (the credentials blob and the stored `oauthAccount` object). "Minimal" means parsability only — Switchy does not validate field presence or types at restore time (capture already did that in AC-1.4, and Switchy treats post-capture corruption as the only failure mode here). If either parse fails, the swap is aborted before any target file is touched, and the error names which snapshot file was corrupt.

### US-3 — Switch to an Official provider with no captured snapshot

**As a** user who has an Official provider without a captured snapshot (e.g. a legacy entry, or one they haven't captured yet), **I want to** switch to it, **and** have Switchy fall back to today's behavior so I can still use that provider lane.

**Acceptance criteria**

- **AC-3.1** If the target Official provider has no captured snapshot, Switchy runs the existing `write_live_snapshot` path and performs **no** credential file manipulation. `~/.claude/.credentials.json` and `~/.claude/.claude.json` are left untouched.
- **AC-3.2** The provider card for an uncaptured Official entry shows a "no captured account" state with a one-click capture affordance that invokes US-1's flow against the currently live OAuth state. (This is the migration path for existing entries.)

### US-4 — WSL mirror stays symmetric

**As a** user who has configured a WSL Claude mirror directory, **I want to** have the credential swap also applied to the WSL `.claude/` target, **so that** running `claude` inside WSL picks up the same Anthropic account as Windows did.

**Acceptance criteria**

- **AC-4.1** When `get_claude_mirror_override_dir()` returns `Some(dir)`, the credential swap writes `{dir}/.credentials.json` and updates the `oauthAccount` block in `{dir}/.claude.json` (or `{dir}/claude.json` using the same legacy-filename fallback the existing mirror uses in `live.rs:748-755`) in addition to the Windows targets.
- **AC-4.2** If the mirror directory is unreachable (UNC path fails, WSL not running) the credential swap to the Windows target still succeeds. The mirror failure is logged at `warn` level, following the precedent in `live.rs:763-768`. It does not fail the switch command.
- **AC-4.3** If the mirror target's `.claude.json` exists but cannot be parsed, credential swap for the mirror is skipped (same failure mode as the existing provider-field mirror in `live.rs:763-768`). The Windows target still swaps.
- **AC-4.4** The existing provider-field mirror behavior (env subset + `model` + `permissions` + `effortLevel`) is preserved unchanged. This spec adds credential writes alongside it, not instead of it.

### US-5 — Remove a captured account

**As a** user who no longer wants a second Anthropic account managed by Switchy, **I want to** clear a provider's captured snapshot without deleting the provider, **so that** the provider reverts to US-3 behavior.

**Acceptance criteria**

- **AC-5.1** The provider card offers a "clear captured account" action. Invoking it removes that provider's snapshot from `~/.switchy/`. The provider row remains.
- **AC-5.2** Clearing the captured snapshot does not modify `~/.claude/` state. If that provider is currently active and Claude Code is logged in with its account, the user remains logged in — Switchy simply forgets the snapshot.
- **AC-5.3** Deleting the Official provider entirely (existing Delete action) also deletes the captured snapshot for that provider in the same transaction.

## Failure-mode summary (cross-story)

| Failure | Spec behavior | Driving AC |
|---|---|---|
| `.credentials.json` missing at capture | Capture fails, localized error | AC-1.3 |
| `oauthAccount` missing/malformed at capture | Capture fails, localized error | AC-1.4 |
| Snapshot's credentials blob no longer parses as JSON | Swap aborted, target untouched | AC-2.4 |
| Target file locked by running Claude Code | Swap fails, target untouched, localized error | AC-2.3 |
| WSL mirror path unreachable | Warn + continue Windows-only | AC-4.2 |
| WSL mirror `.claude.json` unparseable | Skip mirror credential update, continue Windows-only | AC-4.3 |
| No captured snapshot on switch | Fall through to today's behavior, no error | AC-3.1 |

## Out of scope (deferred to later versions)

- **macOS Keychain.** ming86/cc-account-switcher handles macOS via `security find-generic-password -s "Claude Code-credentials"`. v1 is Windows + WSL only. macOS capture/restore is a v2 decision — deferred because (a) macOS is not this user's daily platform and (b) Keychain manipulation requires a different privilege/UX path than file I/O.
- **Encryption of the snapshot store.** v1 stores captured `.credentials.json` in plaintext under `~/.switchy/` with the same filesystem permissions the user's `~/.claude/.credentials.json` already has. Cross-machine sharing of snapshots and at-rest encryption are deferred.
- **More than N accounts.** Switchy already supports arbitrarily many provider rows. No cap is introduced by this spec; the feature works for 1 captured account, 2, or N. "Two" is the primary motivating case but the design should not hardcode it.
- **Codex / Gemini / OpenCode / OpenClaw equivalents.** Each of those apps has a different auth model. v1 ships Claude only.
- **Automating `claude /login`.** Switchy does not launch or automate the Claude Code login flow. The user must log in once per account before invoking capture.
- **Snapshot migration across Switchy versions.** If a future Switchy version changes the snapshot storage layout, that migration is out of scope for v1.

## Success criteria (end-to-end check)

The feature is correct when:

1. Fresh user logs into Claude Code with account A, runs Switchy capture on the default Official provider → card shows `a@example.com`.
2. User runs `claude /login` with account B, adds a new Official provider in Switchy, runs capture on it → card shows `b@example.com`.
3. User clicks switch on the A provider → Claude Code (on next launch) is account A. User clicks switch on B → account B.
4. Same 3 steps performed with mirror configured also update the WSL `.claude/` target. Verification: after switching to provider B on the Windows side, running `claude /status` (or equivalent identity-showing command) inside WSL reports account B's `emailAddress`, not A's. Running `cat ~/.claude/.claude.json | jq .oauthAccount.emailAddress` inside WSL returns the same email as on Windows.
5. The Fork Polish Punch List items in `BACKLOG.md` remain doable — nothing in this spec blocks them.

## Review dismissals

Iteration 1 review findings rebased per the stage-locating rule in `spec_quality_guide.md` §Cross-Stage Rules:

- **"Define `{settingsOk, credentialOk}` return shape for switch command"** — Stage 2 (design) concern, not requirements. AC-2.2 states the user-visible outcome (partial success surfaces as a toast so the user can retry credential step); the interface shape is the design doc's job.
- **"Fix `ProviderCard.tsx:60-80` to `:60-81`"** — off-by-one on a line reference. P2 cosmetics, does not block design.
- **"Define i18n keys and default strings for error messages"** — Stage 3 (tasks) concern. Requirements list the error conditions that must surface to the user; the strings and key names are implementation detail.
- **"Specify error message text (e.g., 'Snapshot for provider X is corrupted')"** — same. Tasks stage.

## Open questions (resolve during design, not here)

- **O-1.** Exact shape of the `oauthAccount` block beyond `emailAddress` and `accountUuid`. The ming86 README does not document it (only says to preserve-and-merge the section). **Design-stage action:** inspect a real `~/.claude/.claude.json` on the user's machine and document observed fields with a `# shape as of YYYY-MM-DD` note. The capture/restore logic still treats the object as opaque-except-identity-fields; observed shape is for documentation only.
- **O-2.** Whether the mirror target's `.claude.json` file name is `.claude.json` specifically or also has the same "try `claude.json` as legacy" fallback. The existing mirror logic uses the legacy fallback for `settings.json → claude.json` (`live.rs:748-755`). Design stage must confirm whether the same fallback applies for the `oauthAccount`-bearing file on the mirror target, which is a different file.
- **O-3.** Whether Switchy should attempt the credential swap for the currently-selected Official provider automatically on app startup (to recover from a manual `~/.claude/` edit the user made outside Switchy), or only on explicit switch. Design stage decides; default leaning is "only on explicit switch" to avoid surprise overwrites.
