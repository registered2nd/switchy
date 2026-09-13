# Internal Changelog

Repo-level changes that affect how future sessions and agents work in this project.
Audience: you and future Claude/Codex sessions. Versioned independently from the product `CHANGELOG.md` — internal milestones track capability of the repo (specs, skills, conventions, build identity), not product releases.

## [i1.0.3] — 2026-09-12 — Kimi's contract is on disk; schema v7; the hermetic rule covers the Codex mirror

### Added
- **`brief_archive/kimi_frontend_checklist.md`** — the contract the Kimi frontend was built against: the provider's `{ config, credentials }` shape, the IPC settings fields, and Kimi's `config.toml` grammar as verified against kimi-code 0.26.0. Read it before changing Kimi on either side of the IPC boundary.
- **`src-tauri/tests/kimi_provider.rs`** — switch-and-backfill round trip for Kimi, and rejection of a provider with no config text. `tests/support.rs` now clears `.kimi-code` between tests.

### Changed
- **Database schema is v7**: `enabled_kimi` on `mcp_servers` and `skills`. `migrate_v6_to_v7` skips a table that does not exist, so partial legacy fixtures (the v4 pricing test seeds no `skills` table) still migrate.
- **The auto-detected Codex mirror directory is suppressed under `SWITCHY_TEST_HOME`**, the same rule i1.0.2 applied to the Claude mirror, so Codex tests cannot write into a real WSL `~/.codex`.

## [i1.0.2] — 2026-08-16 — The test suite is actually hermetic, and the CI gates pass again

### Fixed
- **Tests were writing into the real WSL home.** `SWITCHY_TEST_HOME` redirects everything derived from `get_home_dir()`, but not the mirror directory, which `get_claude_mirror_override_dir()` *auto-detects* by probing for a WSL distro. On any machine with WSL, swap tests wrote credentials and identity into `\\wsl$\...\home\<user>\.claude` — and three assertions had been failing there for months, because "no mirror configured" tests were silently running as mirror tests. The auto-detected default is now suppressed whenever `SWITCHY_TEST_HOME` is set; tests wanting a mirror set one explicitly. Recipe for spotting the class of bug: `LEARNINGS.md`.
- **`cargo clippy -- -D warnings` failed on `main`.** `deeplink/parser.rs` compared the URL scheme against the same literal twice — leftover from the de-fork rename, which replaced both branches of a dual-scheme check with the same string. Fixed along with the error message that read "expected 'switchy' or legacy 'switchy'".
- **`cargo fmt --check` (71 diffs) and `pnpm format:check` (28 files) both failed on `main`**, so no CI run could pass regardless of the change under test. Both are clean now; keep them that way rather than letting the debt rebuild.

### Changed
- **Spec `specs/official_multi_account/design.md` §Data Sources rows 2 and 4 corrected.** They specified selecting the live `.claude.json` by probing which candidate file exists, which is the defect shipped as the 1.0.8 fix. Resolution is now by rule. Treat the rest of that spec as history, not instructions.

## [i1.0.1] — 2026-06-13 — Build requires pnpm 11 `allowBuilds` approval

### Changed
- **Building under pnpm 11.5.2 now requires explicit build-script approval.** `pnpm-workspace.yaml` gained an `allowBuilds:` map (`esbuild: true`, `msw: true`) — pnpm 11's gate that blocks dependency build scripts by default. Builds also need `CI=true` throughout (non-TTY `node_modules` sync). Full recipe in `LEARNINGS.md`. Without these, `pnpm tauri build` fails before compiling (`ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY`, then `ERR_PNPM_IGNORED_BUILDS`).

## [i1.0.0] — 2026-04-17 — De-fork: Switchy stands on its own

### Changed
- **Repo identity cut from upstream `cc-switch`.** Bundle (`com.switchy.desktop`), binary (`switchy.exe`), Rust `[lib]` name, Cargo authors + repository URL, `package.json` author, and `git origin` all now name Switchy. Upstream `farion1231/cc-switch` kept as the `upstream` remote for cherry-picks.
- **App data namespace renamed.** `~/.cc-switch/` → `~/.switchy/`, `cc-switch.db` → `switchy.db`. Supersedes the 2026-04-07 `DECISION_LOG.md` entry that parked this for continuity.

### Added
- **`paths` module** (`src-tauri/src/paths.rs` + `src/lib/paths.ts`) — single source of truth for app-dir and DB-file names, including `LEGACY_*` constants used by the migration shim. Replaces hardcoded literals across config, panic hook, settings, env manager, database, and TS directory hooks.
- **Migration shim** (`src-tauri/src/migrate_paths.rs`) — one-shot startup rename of `~/.cc-switch/` → `~/.switchy/` (plus DB file). Idempotent; no-ops when new dir already exists or neither exists; warns when both exist. Unit-tested for the four scenarios. Intended for removal after one release cycle.
- **`SWITCHY_TEST_HOME` env var** for test-home override, with fallback to the legacy `CC_SWITCH_TEST_HOME` so in-flight tests keep passing during transition.
- **`specs/de_fork/{requirements,design,tasks}.md`** — the spec this work was executed against.

### Removed
- Dual-scheme deep-link handler (kept only `switchy://`).
- `"Fork polish punch list"` from `BACKLOG.md`, replaced with a tighter post-de-fork punch list (smoke test, WSL sync verification, optional polish).
