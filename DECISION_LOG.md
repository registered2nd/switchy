# Decision Log

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

