# Decision Log

## 2026-07-05 — macOS Keychain capture/swap implemented (the deferred v2), superseding the 2026-04-16 macOS-out-of-scope call

- Decision: Implement Claude-account **capture and swap on macOS** by reading/writing Claude Code's login-Keychain item (`Claude Code-credentials`) instead of the `~/.claude/.credentials.json` file used on Windows/Linux. Closes the macOS gap the 2026-04-16 entry deferred to "v2."
- Why:
  1. Capture hard-failed on macOS with "No Claude Code login found" even when logged in — Claude Code on macOS stores its OAuth blob in the Keychain, not a file; swap wrote a file Claude Code never reads, so switching silently didn't take. The feature was effectively unusable on the platform.
  2. The read half already existed in-tree: `services::subscription` reads the same `Claude Code-credentials` item via the `security` CLI. Low incremental cost, proven pattern.
- Approach:
  - New macOS-only `services/claude_account/keychain.rs` shells out to `security` (no new dependency, consistent with `subscription.rs`). Capture, swap-restore, and switch-away sync route through platform-abstracted `read_live_credentials` / `write_live_credentials` helpers in `mod.rs`.
  - Writes update the existing Keychain item **in place** (`add-generic-password -U` against the item's own `acct`) so a re-capture or account swap overwrites the current login rather than duplicating it.
  - Tests bypass the Keychain via `SWITCHY_TEST_HOME` (the Keychain is a global side-channel the file redirect can't sandbox) so they stay hermetic.
- Consequence:
  - Windows/Linux behavior unchanged (file path preserved).
  - Shipped as **1.0.7** on branch `fix/macos-keychain-capture`; macOS CI build compiled clean; DMG released to `switchy-dist`. **Not yet runtime-verified on a real Mac** — see `NEXT_SESSION.md`.
  - Known macOS UX caveat: reading the Keychain item may raise a one-time "security wants to access…" prompt (choose Always Allow), since Switchy isn't signed under Claude Code's identity.

## 2026-06-13 — Credential mirror: bidirectional + health-aware + account-guarded, superseding one-way

- Decision: Replace 1.0.5's one-way (live → mirror) credential-mirror watcher with a **bidirectional, health-aware, account-guarded reconciler** (1.0.6). It propagates the freshest *valid* `claudeAiOauth` bundle in whichever direction is stale, but only between sides that are the **same account** (matched by `oauthAccount` UUID); it never propagates a dead/blanked bundle, and it syncs only the `claudeAiOauth` block (preserving each machine's `mcpOAuth`).
- Why:
  1. **One-way re-armed the race instead of ending it.** The 2026-05-06 design assumed Win is the sole refresher and WSL a passive downstream consumer. In practice WSL runs its own (multiple) long-lived Claude Code processes that refresh on their own ~8h schedule. The one-way watcher continuously fed WSL a fresh token, keeping it eligible to win the refresh race; when WSL won, Win was left on a dead token (401), and the one-way copy then propagated Win's *blanked* file onto WSL too — killing both. Turning Switchy off (no re-arming → WSL falls out of the chain) is what made the 401s stop, which is the proof the watcher was the driver.
  2. **A running Claude Code session re-reads `.credentials.json` per request** — demonstrated by Switchy's own mid-session account-switch working immediately. So writing the winner's fresh bundle back to the loser's file auto-recovers it; the file layer *is* the right place to fix this, provided the sync is bidirectional. (See `LEARNINGS.md`.)
  3. **Not naive last-writer-wins (the BACKLOG #10 sketch).** mtime-newest would copy a blanked failed-refresh file (it's the newest write) over a good one. Freshness must be judged by `expiresAt` among *valid* bundles only, and direction gated by same-account — otherwise a mid-switch transient or a deliberate per-machine login would get clobbered/reverted.
- Consequence:
  - 1.0.6 ships the reconciler (`services/credential_mirror.rs`, full rewrite, 13 unit tests). `start()` wiring in `lib.rs` unchanged.
  - The swap-time `.credentials.json` mirror in `mod.rs` is retained (first-swap population); BACKLOG #12's "keep it" call stands.
  - BACKLOG #10 is closed (built, beyond the naive last-writer-wins it described).
  - Residual: the rare sub-second simultaneous-refresh tie still briefly fails the loser, which then auto-heals on its next request (~poll interval). True zero-blip needs a single-refresher auth-broker (out of scope; Switchy's proxy layer could host it).

## 2026-05-28 — Fork ships its own unsigned macOS DMG; tracks current installers in git; retires upstream cc-switch versioning

- Decision: (1) Build the fork's macOS DMG via a new on-demand **unsigned** workflow (`build-macos.yml`, hdiutil-packaged, Apple Silicon only), separate from the inherited `release.yml`. (2) **Track the current version's installers in git** (`installers/` with a Vtype-style re-include `.gitignore`), reversing the 2026-04-19 don't-commit-installers decision. (3) Treat upstream cc-switch's `3.13.0` versioning as **retired** — the fork owns its 1.0.x line.
- Why:
  1. **The inherited release.yml can't run on this fork.** `release.yml` + the `v3.x` tags came from upstream cc-switch and require Apple Developer signing/notarization secrets the fork doesn't possess. Rather than buy a $99/yr Developer ID for a personal fork, an unsigned hdiutil DMG (Vtype's pattern) gives a working macOS artifact for zero cost. Gatekeeper is cleared per-install with `xattr -dr com.apple.quarantine /Applications/Switchy.app`.
  2. **Apple Silicon only, not universal** — the user's machines are Apple Silicon; the x86_64 slice doubled runner compile time for no benefit.
  3. **Tracking installers reverses 2026-04-19 deliberately.** That decision kept installers out of git to avoid binary bloat. The countervailing pull now: the user wants the current cross-platform installers gathered and visible in the repo like Vtype does. Mitigated by the re-include pattern tracking ONLY the current version — old versions are pruned by postbuild and never committed, so bloat stays bounded to ~3 files.
  4. **Retiring 3.13.0 removes version confusion.** 3.13.0 (and the stray 1.1.0) outrank 1.0.5 numerically but are older and not the fork's line. All three version-of-record files agree on 1.0.x; the fork's history starts fresh from 1.0.0.
- Consequence:
  - `build-macos.yml` is the fork's macOS build path; trigger via Actions → Run workflow. Re-runs are fast once the cargo cache is warm.
  - `release.yml` left intact but **dormant** — not deleted, in case upstream-style signed releases are wanted later (would need a Developer ID + secrets).
  - `scripts/postbuild.mjs` is now the single promote-and-prune mechanism for Windows installers; `installers/` is the canonical distributable location.
  - macOS support is verified by **source inspection** (cross-platform `get_home_dir`, cfg-gated WSL code) — **not** by launching the build on a real Mac. First actual Mac launch is the outstanding verification.

## 2026-05-06 — Credential mirror watcher: ship the file watcher, one-way live → mirror

> **Superseded by 2026-06-13 (bidirectional + health-aware reconciler)** — one-way kept WSL armed to win the refresh race and then propagated the loser's blanked file, killing both sides. Replaced with a bidirectional, account-guarded, health-aware reconcile. The diagnosis of the rotation race itself (why sharing one credential across the WSL boundary needs sustained sync) remains correct.

- Decision: Ship a `notify`-backed FileSystemWatcher on `~/.claude/.credentials.json` that copies to `mirror_dir/.credentials.json` on every change. **One-way live → mirror only**, not bidirectional. Retain the existing swap-time `.credentials.json` mirror in `services/claude_account/mod.rs` for the first-swap-into-an-account case (before the watcher has anything to fire on).
- Why:
  1. **The 2026-05-04 entry's conclusion was wrong, and this session's forensic data corrects it.** That entry decided the post-switch refresh race wasn't supported by observation, based on a 20-minute manual-cp re-test where both sides held identical sha256. Today's WSL 401 traced to exactly the rotation race: Win refreshed `R0 → R1` over the evening, WSL was left holding R0, WSL's later refresh attempt got rejected, Claude Code blanked WSL's refresh_token field (length 0). The 2026-05-04 test was within access-token validity — neither side *needed* to refresh in 20 minutes, so the race couldn't manifest. SESSION_LOG today has the full timeline.
  2. **OAuth refresh tokens are server-rotated single-use.** Once two filesystem stores diverge from a shared starting credential, whichever side refreshes first invalidates the other side's refresh token. Sharing `.credentials.json` across the WSL boundary is structurally doomed without sustained sync — exactly what the watcher provides.
  3. **One-way (not bidirectional) is the right shape today.** Win is the active refresher in normal use (the user runs Claude Code there continuously); WSL is the consumer. One-way live → mirror keeps WSL strictly downstream so its Claude Code never has to attempt a refresh on its own. Bidirectional last-writer-wins works in theory but adds a way for WSL's spurious refresh attempt to overwrite Win's good state. Defer until the Win-side failure mode actually bites (BACKLOG #10).
  4. **The swap-time `.credentials.json` mirror is still needed for first-swap.** The watcher only fires on changes after the live file already exists. The first time a user switches into an account, the swap-time mirror is what populates the WSL side. Removing it would force a one-app-restart wait for the watcher's startup one-shot to copy.
- Consequence:
  - 1.0.5 ships the watcher. `services::credential_mirror::start()` called from `lib.rs` setup hook on every app launch; logs each mirror under `[credential_mirror]` to `switchy.log`.
  - The 2026-05-04 decision's "do not build a watcher" guidance is **superseded** by this entry. The diagnostic logging that 2026-05-04 added remains useful for first-swap-failure diagnosis and is retained.
  - BACKLOG #9 (verify mirror via diagnostic logging) is closed by this session — the mirror IS firing; the bug was downstream rotation drift, not the swap-time write.
  - New BACKLOG items added: bidirectional last-writer-wins (#10), late-appearing watch dir (#11), and a deferred reconsideration of whether the swap-time `.credentials.json` mirror should be dropped now that the watcher exists (#12 — current call: keep it).

## 2026-05-03 — Patch-bump (1.0.3) for the WSL mirror fixes, not minor-bump (1.1.0)

- Decision: The 2026-05-02 → 2026-05-03 ship is **1.0.3**, not 1.1.0. Treat the unreleased committed fixes (`08f19e2c` switch-away sync, plus the four hover/overlay commits `4c9256bf`/`d2f365fc`/`48b10153`/`2965fd84`) as the logical 1.0.1 and 1.0.2 even though they were never tagged or released, and bump to 1.0.3 for everything that landed today.
- Why:
  1. **Nothing here is genuinely new behavior.** WSL mirror auto-detection, home-root `.claude.json` mirror, quota cache invalidation on switch, refresh pill clickability — every change is making something that was supposed to work in 1.0.0 actually work. None of them give the user a capability the 1.0.0 release notes didn't already advertise.
  2. **Initial bump to 1.1.0 was reflexive.** I framed "auto-detection without configuration" as a feature; the user pushed back that it's just removing a configuration step that was supposed to be optional anyway. Same for home-root mirror — making the existing mirror feature reach the file Claude Code actually reads.
  3. **The committed-but-unreleased fixes between 1.0.0 and today represent real semver patch increments.** Six commits since `605e0412`. Pretending they're all part of "1.0.0" undercounts; pretending they're each a release overcount. Treating them as logical 1.0.1 / 1.0.2 lets 1.0.3 actually number the third patch since 1.0.0 instead of compressing six fixes into one bump.
- Consequence:
  - 1.0.1 and 1.0.2 are never tagged, never released, never carry CHANGELOG entries — they only exist as logical reference points to make 1.0.3 numerically honest.
  - Future bug-fix-only ships continue patch-bumping (1.0.4, 1.0.5, ...). Minor bump is reserved for genuinely new user-facing capability — first time a feature does something the prior release explicitly couldn't.
  - All three version-of-record files (`package.json`, `tauri.conf.json`, `Cargo.toml`) bumped together; CHANGELOG entry header matches.

## 2026-05-04 — Credentials mirror sync: ship diagnostic logging, defer remediation until logs reveal actual behavior

- Decision: Add explicit info-level logging around the Claude credentials mirror write path in `services/claude_account/mod.rs` (`mirror cred write START` + `OK`-with-mtime), but do NOT add a file watcher, manual sync buttons UI, or any other remediation until diagnostic logs from a real provider switch on a rebuilt Switchy reveal what the auto-mirror is actually doing.
- Why:
  1. **The post-switch refresh race theory wasn't supported by observation.** Hypothesis was that Windows and WSL Claude Code each refresh OAuth tokens independently after a swap, racing on the rotated refresh_token and producing the `401 Invalid authentication credentials` the user saw on WSL. Empirical re-test 20 minutes after a manual `cp` of `.credentials.json` showed both sides held identical sha256 hashes — neither Claude Code instance had refreshed unprompted. Building a watcher or buttons against a failure mode that doesn't reproduce is premature.
  2. **The auto-mirror's actual failure mode is unconfirmed.** The existing `swap applied with mirror` log fires unconditionally when the warnings vec is empty; we have no signal from the existing code about whether the credentials write step (vs the `.claude.json` or `settings.json` writes that share the same swap path) actually completed. the user's observation that WSL stayed stale through a switch is not yet reconciled with the function returning Ok and no warning. Logging is the cheapest way to disambiguate.
  3. **Remediation paths fork on what the logging reveals.** If START fires but no OK and no warning: silent error in `atomic_write` against UNC paths — targeted fix. If both fire and target mtime updates but the user still sees stale: it's a Claude-Code-side caching issue, not a Switchy bug. If neither fires: the function is bailing earlier than expected (`get_claude_mirror_override_dir` returning None? `swap_if_captured` gating?). Each path is a different fix, and shipping a generic watcher hides which one applies.
- Consequence:
  - Manual sync UI infrastructure built mid-session was reverted: `services/claude_account/sync.rs` deleted, `ClaudeCredentialsMirrorPanel.tsx` deleted, 3 Tauri commands removed, AuthCenterPanel reverted to pre-session state. Diagnostic logging in `mod.rs` retained.
  - Next Switchy rebuild + provider switch is the trigger to read the logs and decide remediation.
  - If diagnostic logs confirm the auto-mirror is working as advertised, the buttons remain unnecessary and the WSL 401 had a different root cause (most likely Claude-Code-side caching of pre-swap credentials before the user typed the next command).

## 2026-04-20 — Abandon snapshot OAuth refresh (BACKLOG #5 and #6)

- Decision: Do not build `snapshot_oauth_refresh` (BACKLOG #5, the on-demand token refresh for captured-but-not-current accounts) or the periodic background refresh loop (BACKLOG #6). Captured-card quota pills showing "Session expired" past the access-token ceiling is the correct user-facing behavior.
- Why — two independent reasons, either sufficient on its own:
  1. **Anthropic's April 2026 "Authentication and credential use" policy explicitly prohibits it.** Verbatim: *"OAuth authentication used with Free, Pro, and Max plans is intended exclusively for Claude Code and Claude.ai. Using OAuth tokens obtained through Claude Free, Pro, or Max accounts in any other product, tool, or service is not permitted and constitutes a violation of the Consumer Terms of Service."* BACKLOG #5, even in its carefully-scoped user-initiated-only variant (5b), uses a Pro/Max refresh_token from Switchy — not from Claude Code — and the policy language names this exact case.
  2. **Cloudflare WAF actively blocks the endpoint for non-browser clients** with the Claude Code Console client_id (`9d1c250a-e61b-44d9-88ed-5944d1962f5e`). This isn't a hypothetical fragility — it's deployed enforcement. Tracked in [anthropics/claude-code#47754](https://github.com/anthropics/claude-code/issues/47754); the same block broke Hermes PKCE refresh ([NousResearch/hermes-agent#6347](https://github.com/NousResearch/hermes-agent/issues/6347)). Even if the policy changes, the engineering path is a moving target.
- Consequence:
  - `specs/snapshot_oauth_refresh/requirements.md` is retained as a historical artifact with an abandonment note at the top. Design and tasks stages are not written.
  - The 2026-04-19 entry below referenced #5 and #6 as "future improvements that extend the window." Those paragraphs are **superseded by this entry** — the window cannot be extended within ToS, full stop. The 8h access-token / ~24h refresh-token ceiling is the feature ceiling, not a lower bound to work up from.
  - If a future session considers similar "help Switchy do more with captured credentials" features, the first check is: does it make any authenticated request using Pro/Max OAuth credentials from a non-Claude-Code-non-Claude.ai client? If yes, it's ToS-prohibited, regardless of how thin or user-initiated the path is.
  - The existing captured-snapshot **quota read** path (which uses the captured `access_token`, not `refresh_token`, against Anthropic's usage endpoint) is a milder version of the same question but is not affected by this decision — it uses tokens while they're still valid, does not call the OAuth refresh endpoint, and is the feature-parity story for BACKLOG #5 of the 2026-04-19 multi-account plan which was retired when switch-away sync shipped. Leave it alone; flag if/when future policy updates extend the prohibition to usage-endpoint reads.

## 2026-04-19 — OAuth token lifetime bounds the multi-account feature

- Constraint:
  - Anthropic Claude Code OAuth tokens have a short effective lifetime.
    Access tokens: **~8–12h**. Refresh tokens: **~24h** practical ceiling
    (inferred from [anthropics/claude-code#42904](https://github.com/anthropics/claude-code/issues/42904)
    — subscription users re-login daily regardless of tooling). Rotation
    appears aggressive — each refresh likely invalidates the prior
    refresh_token.
- Consequence for the captured-account design:
  - The multi-account feature can guarantee switching within **~8h** of the
    most-recent refresh on an account, and is **unreliable past ~24h**. For
    dormant accounts (not used in >24h), switching back will require
    re-login through Claude Code — no local state management fixes this.
  - Therefore: future improvements should be framed as **extending the
    practical window**, not achieving indefinite dormancy.
- Implications for future work (tracked in `BACKLOG.md`):
  - **Switch-away sync** (update outgoing provider's snapshot from live
    creds before writing the incoming provider's snapshot). Stops us from
    silently discarding the newest refresh_token on every switch. This is
    the cheapest buy — roughly doubles the realistic dormancy window
    because each snapshot carries the freshest token that account has
    ever produced.
  - **Snapshot OAuth refresh on demand** (when quota query / switch sees
    an expired access_token but a present refresh_token, hit
    `console.anthropic.com/v1/oauth/token` with grant_type=refresh_token,
    persist new tokens into the snapshot). Works around the access-token
    ceiling, not the refresh-token ceiling. Fragile — Anthropic has been
    deprecating third-party OAuth usage.
  - **Periodic background refresh loop** (Switchy keeps captured accounts
    warm by refreshing them before they expire). Theoretically pushes the
    window toward "indefinite" but hammers Anthropic's endpoints and
    assumes OAuth continues to be available to third-party apps. Not
    recommended as a priority.
- Where NOT to look for fixes:
  - Snapshot encryption, different file layouts, more atomic writes — all
    orthogonal to the token-lifetime problem.
  - `claude setup-token` produces a 1-year token but is a different auth
    mode (automation/headless). Switchy cannot transparently upgrade a
    captured subscription OAuth to a setup-token; the user would need to
    re-auth via that flow explicitly.

## 2026-04-18 (evening) — NSIS PREINSTALL hook does NOT force-kill the running app

- Decision:
  - `src-tauri/nsis/installer-hooks.nsh` runs `taskkill /F /IM switchy.exe /T` in `NSIS_HOOK_PREUNINSTALL` only. The `NSIS_HOOK_PREINSTALL` hook is deliberately a no-op (no taskkill).
- Why:
  - The first implementation of BACKLOG #4 put the taskkill in both PREINSTALL and PREUNINSTALL. Tauri's generated `installer.nsi` runs PREINSTALL *before* its built-in `CheckIfAppIsRunning` macro — so killing in PREINSTALL meant the macro found nothing running and the "Switchy is running, close it?" prompt never fired on upgrade. the user flagged this as regressing UX: silent auto-kill on upgrade is surprising and worse than the old prompt-then-maybe-fail behavior.
  - Uninstall is different: the user has already clicked "uninstall", the decision is already made, and a prompt at that point just adds friction. Force-killing on PREUNINSTALL is fine.
  - The residual BACKLOG #4 failure mode (install silently skips `switchy.exe` replacement when the user dismisses the prompt without actually closing the app) is accepted — fixing it properly needs a hook *between* `CheckIfAppIsRunning` and `File "${MAINBINARYSRCPATH}"`, which the Tauri template doesn't expose. Options if this re-surfaces: (a) full custom NSIS template, (b) add a post-install verification that retries the file copy.
- Consequence:
  - Future sessions touching `installer-hooks.nsh` should not add a PREINSTALL taskkill without understanding this trade-off.
  - If the silent-skip failure returns, escalate to a full template override rather than re-adding the aggressive kill.

## 2026-04-18 — Drop T-5.2 WSL validation from the multi-account ship

- Decision:
  - T-5.2 (WSL mirror end-to-end walkthrough) is skipped for this ship. The multi-account feature ships after BACKLOG polish items #4–#7 are addressed; WSL validation is not a precondition.
- Why:
  - T-5.1 proved the core capture → swap → `/status` contract works end-to-end on Windows. The WSL mirror code paths are already covered by Rust unit tests in `services::claude_account::tests` (happy-path `AppliedWithMirror`, `PartialMirror:parse`, `PartialMirror:unreachable` — AC-4.1 through AC-4.3).
  - The polish items surfaced in the T-5.1 run (NSIS installer, per-account quota, zh-leak, tray click) are visible to every user on every install and are higher-impact than WSL-specific validation.
  - Stacking T-5.2 on top delays ship for a feature-path that only the user's WSL setup exercises; better to ship and let any WSL regression surface as its own bug report.
- Consequence:
  - `specs/official-multi-account/tasks.md` T-5.2 is marked SKIPPED with its checklist retained for reference.
  - If a WSL mirror regression is reported post-ship, it becomes a fresh spec session, not a blocker on 3.13.0.

## 2026-04-17 - De-fork Switchy: supersede upstream namespace decision

- Decision:
  - Switchy is its own project, not a fork. The `de-fork` spec
    (`specs/de-fork/`) renamed upstream `cc-switch` / `CC Switch` /
    `com.ccswitch.desktop` identifiers, the repository remote, and the app
    data directory (`~/.cc-switch/` → `~/.switchy/`, `cc-switch.db` →
    `switchy.db`). A one-shot migration shim
    (`src-tauri/src/migrate_paths.rs`) renames existing installs in place
    on startup.
- Why:
  - Fork status was holding back the project's identity and made future
    divergence harder. Native multi-account switching (active spec) would
    have deepened the branching debt.
  - The 2026-04-07 decision below (keep upstream namespace for continuity)
    is now obsolete; the migration shim supplies the continuity by
    renaming the directory rather than leaving it under an upstream name.
- Consequence:
  - The old `farion1231/cc-switch` remote is preserved as `upstream`
    for cherry-picking; `origin` now points at `registered2nd/switchy`.
  - Users upgrading from the pre-rename build have their data auto-migrated
    on first launch. The shim is idempotent; ship for one release then
    remove.
  - Historical entries (SESSION_LOG, CHANGELOG entries describing shipped
    versions, the 2026-04-07 entry below) are preserved verbatim and
    intentionally still mention the upstream name — they describe past
    state.

## 2026-04-07 - Keep existing app data namespace for continuity

*(Superseded 2026-04-17 by the de-fork decision above. Kept verbatim as history.)*


- Decision:
  - Keep the existing internal storage/config namespace (`~/.cc-switch`, related storage keys, and inherited profiles) for now.
- Why:
  - The user explicitly wanted to preserve the current profiles/state rather than isolate the fork immediately.
  - Renaming the app identity is sufficient for installation/branding separation, while keeping data continuity avoids accidental loss of existing provider state.
- Consequence:
  - `Switchy` still reads the existing `CC Switch` app data/state.
  - Uninstalling the old app does not remove those profiles, which is acceptable and currently desired.

## 2026-04-07 - Mirror WSL Claude by provider-field sync, not full-file sync

- Decision:
  - The WSL mirror target should only receive provider-related Claude fields, not a full-file overwrite.
- Why:
  - Full mirroring copied Windows-only hooks/status-line commands into WSL and broke WSL runtime behavior.
  - The practical goal is shared provider switching across Windows and WSL without destroying machine-specific local wiring.
- Consequence:
  - Mirror target preserves local hooks/status-line/plugins and only updates provider-related fields.
  - If the mirror file cannot be parsed, mirror sync is skipped rather than falling back to a destructive overwrite.

## 2026-04-10 - Absorb cc-account-switcher capability into Switchy as native feature

- Decision:
  - Rather than adopting `ming86/cc-account-switcher` as a separate bash tool, port its credential-swap capability into Switchy as a first-class feature, exposed through the existing provider UI.
  - Concretely: when an "Official" provider is added, Switchy captures a snapshot of the active Claude Code OAuth state. When that provider is enabled, Switchy restores that snapshot to `~/.claude/` (and mirrors to the WSL `.claude/` target if configured). This allows two or more "Official" providers to represent two distinct Anthropic accounts, switchable like any other provider.

- Why:
  - Stock cc-switch (and current Switchy) only manipulates env vars / `settings.json` for "Official" providers — it cannot distinguish between two Anthropic accounts because the OAuth credentials live in `~/.claude/.credentials.json` and `~/.claude/.claude.json` (`oauthAccount` field), neither of which Switchy currently touches.
  - `ming86/cc-account-switcher` already proved the swap pattern works (back up + restore those exact files), but it is a bash script targeting macOS/Linux/WSL only and lives outside Switchy's UI/state model. Running it in parallel would split state across two tools.
  - Switchy already owns:
    - the provider switching UI and storage
    - the `~/.claude/` write path (Windows)
    - the WSL `~/.claude/` mirror write path (this fork's addition)
    so adding credential-swap is incremental, not architectural.
  - This pushes Switchy further from upstream — the user explicitly wants Switchy to "have its own life" and the WSL mirror was the first such divergence; multi-account credential swap is the second. Future divergence from upstream is acceptable.
  - The user is on Pro plan and unlocking 1M context requires either Max upgrade or enabling extra-usage billing on a Pro account. Managing two separate Anthropic accounts cleanly (each with its own subscription level / billing posture) is the underlying need that motivates this feature now rather than later.

- Consequence:
  - Switchy will need a per-provider credential cache under its existing data dir (current namespace: `~/.cc-switch/` — see prior decision to keep the upstream namespace for continuity).
  - The Rust services layer in `src-tauri/services/provider/` will gain credential snapshot/restore alongside the existing live config writers. Both Windows and WSL `.claude/` targets must be handled symmetrically with the existing mirror logic.
  - The "Official" provider concept in the UI grows a "captured account" notion (display email/UUID from `oauthAccount`) so users can tell two Official entries apart.
  - Spec-driven workflow applies: this is non-trivial, so the next session begins at `requirements.md` per `spec-quality-guide.md`. Do not skip stages.
  - Fork polish (renaming residue, end-to-end installer test, namespace isolation decision) is now backlog, not active. It is preserved in `SESSION_LOG.md` 2026-04-10 entry and should be picked up after the multi-account feature lands or when it becomes blocking.

## 2026-04-16 - Plaintext snapshots + macOS deferred to v2 (scope-shaping calls from the spec)

> **macOS half superseded by 2026-07-05** — Keychain capture/swap is now implemented (1.0.7). The plaintext-snapshot decision below still stands.

- Decisions (scope-shaping, made during the requirements/design loop for official multi-account):
  - v1 stores per-provider OAuth snapshots plaintext under `~/.cc-switch/accounts/{provider_id}/`, with `0o600` on Unix and default NTFS user-profile ACL on Windows. No encryption.
  - v1 handles Windows + WSL only. macOS Keychain capture/restore is explicitly out of scope — the Rust code path on macOS returns a clear `AppError` rather than silently succeeding with wrong state.
- Why (documented once here so future sessions don't re-derive):
  - Claude Code's own `.credentials.json` is already plaintext with the same permissions on the same machines; Switchy's snapshot has the same sensitivity. Adding encryption requires a key-management story (where does the key live? is it per-device?) that is out of proportion for a two-account local use case.
  - macOS Keychain uses a different API surface (`security find-generic-password`) than the file-based capture the reference impl uses on Linux/WSL. Implementing both correctly in v1 doubles the test matrix for a platform the user does not run Switchy on daily.
- Consequence:
  - If a future Switchy release needs cross-machine snapshot sync or cloud backup, encryption becomes load-bearing and must be added as a migration (not a retrofit).
  - Adding macOS later is a v2 spec, not an implementation patch — the file-path abstractions and error surfaces must accommodate Keychain semantics.
- Spec pointer: `specs/official-multi-account/requirements.md` "Out of scope" + `specs/official-multi-account/design.md` D-7.

## 2026-04-07 - Rename forked app to Switchy and keep legacy deep-link compatibility

- Decision:
  - Rename the forked app to `Switchy` and switch the public deep-link scheme to `switchy://`, while still accepting legacy `ccswitch://`.
- Why:
  - The fork needed a distinct installed identity and visible branding.
  - Legacy deep-link compatibility avoids breaking existing links and upstream-style imports immediately.
- Consequence:
  - Installer/binary/app identifiers are now `Switchy`.
  - Visible branding is mostly `Switchy`.
  - Legacy deep links still function.

