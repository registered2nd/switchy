# De-fork Switchy — Requirements

## Goal
Switchy is now its own project, not a fork. Strip internal references to the upstream project (`cc-switch`, `CC Switch`, `~/.cc-switch/`, `cc-switch.db`, `com.ccswitch.desktop`) from code, build config, docs, and runtime paths. Historical records (`SESSION_LOG.md`, git history, `CHANGELOG.md` entries describing shipped versions) are preserved verbatim.

This supersedes the 2026-04-07 decision in `DECISION_LOG.md` to keep the upstream data namespace. The supersession is logged, not redacted.

## In scope
1. **Build identity.** Crate lib name, authors, repository URL; `package.json` author; bundle identifier and Flatpak manifest filenames/ids.
2. **Runtime paths.** App config dir renamed `~/.cc-switch/` → `~/.switchy/`. DB file renamed `cc-switch.db` → `switchy.db`. Backups dir, skills dir, crash log, settings file paths follow suit.
3. **Environment variable override.** `CC_SWITCH_HOME` → `SWITCHY_HOME` (if it exists; verify during design).
4. **Source literals.** All `.cc-switch`, `cc-switch.db`, `cc_switch_*`, `com.ccswitch.*`, and user-facing `CC Switch` / `CC-Switch` strings in Rust + TS sources, tests, i18n files, and deep-link plugin paths.
5. **Docs.** `README.md`, `README_ZH.md`, `README_JA.md`, `CONTRIBUTING.md`, `SECURITY.md`, `SUPPORT.md`, `docs/user-manual/**`, `flatpak/README.md`. Any remaining `CHANGELOG.md` references to `~/.cc-switch/` become `~/.switchy/` **only** when describing current behavior; historical version entries are preserved.
6. **Migration shim.** On first launch after this change, if `~/.switchy/` is missing and `~/.cc-switch/` exists, rename the directory and rename `cc-switch.db` → `switchy.db` inside it. Idempotent, one-shot, no data loss. Logged.
7. **Git remote.** Change `origin` from `farion1231/cc-switch` to the user's own repo URL (URL captured during tasks; create the GitHub repo manually or via `gh`).
8. **Forward-looking project docs.** `BACKLOG.md`, `NEXT_SESSION.md`, `specs/**` (except `DECISION_LOG.md` and `SESSION_LOG.md`) updated where they describe current state or intent.

## Out of scope (explicitly)
- `SESSION_LOG.md` entries — append-only history.
- `DECISION_LOG.md` prior entries — history. A **new** entry supersedes the 2026-04-07 namespace decision; old entries stay verbatim.
- `CHANGELOG.md` entries describing past releases (they shipped with `~/.cc-switch/`, so saying otherwise would be false history).
- Rebranding the upstream commits in git log. Git history stays.
- Renaming `target/`, `node_modules/`, or anything vendored.
- Data inside the user's live `~/.cc-switch/` — the migration renames the container, not the contents.

## Success criteria
- `grep -i "cc[-_]switch\|ccswitch"` across `src/`, `src-tauri/src/`, `src-tauri/tests/`, `tests/`, top-level docs returns only (a) things in scope/out-of-scope carve-outs above, (b) the migration shim's legacy detection, (c) the supersession entry in `DECISION_LOG.md`.
- `pnpm tauri build` produces a `switchy` binary and installer with bundle id `com.switchy.desktop`.
- App launches with existing `~/.cc-switch/` present → migrates to `~/.switchy/`, finds all providers/skills/settings intact.
- App launches with no state dir → creates `~/.switchy/` fresh.
- `git remote -v` shows the user's repo, not upstream.

## Non-goals
- Version bump. Keep `3.12.3` or bump by user's choice — not decided here.
- UI redesign or icon change.
- Changing the DB schema or storage keys *inside* the DB (only the filename of the DB).
