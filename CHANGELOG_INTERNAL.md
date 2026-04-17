# Internal Changelog

Repo-level changes that affect how future sessions and agents work in this project.
Audience: you and future Claude/Codex sessions. Versioned independently from the product `CHANGELOG.md` — internal milestones track capability of the repo (specs, skills, conventions, build identity), not product releases.

## [i1.0.0] — 2026-04-17 — De-fork: Switchy stands on its own

### Changed
- **Repo identity cut from upstream `cc-switch`.** Bundle (`com.switchy.desktop`), binary (`switchy.exe`), Rust `[lib]` name, Cargo authors + repository URL, `package.json` author, and `git origin` all now name Switchy. Upstream `farion1231/cc-switch` kept as the `upstream` remote for cherry-picks.
- **App data namespace renamed.** `~/.cc-switch/` → `~/.switchy/`, `cc-switch.db` → `switchy.db`. Supersedes the 2026-04-07 `DECISION_LOG.md` entry that parked this for continuity.

### Added
- **`paths` module** (`src-tauri/src/paths.rs` + `src/lib/paths.ts`) — single source of truth for app-dir and DB-file names, including `LEGACY_*` constants used by the migration shim. Replaces hardcoded literals across config, panic hook, settings, env manager, database, and TS directory hooks.
- **Migration shim** (`src-tauri/src/migrate_paths.rs`) — one-shot startup rename of `~/.cc-switch/` → `~/.switchy/` (plus DB file). Idempotent; no-ops when new dir already exists or neither exists; warns when both exist. Unit-tested for the four scenarios. Intended for removal after one release cycle.
- **`SWITCHY_TEST_HOME` env var** for test-home override, with fallback to the legacy `CC_SWITCH_TEST_HOME` so in-flight tests keep passing during transition.
- **`specs/de-fork/{requirements,design,tasks}.md`** — the spec this work was executed against.

### Removed
- Dual-scheme deep-link handler (kept only `switchy://`).
- `"Fork polish punch list"` from `BACKLOG.md`, replaced with a tighter post-de-fork punch list (smoke test, WSL sync verification, optional polish).
