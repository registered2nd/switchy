# De-fork Switchy — Tasks

Order matters. Path/build changes first (they determine what the code compiles to), then migration shim, then call-site sweep, then docs, then remote.

## Phase 1 — Central constants & build identity

- [ ] **T1.** Create `src-tauri/src/paths.rs` with the constants listed in `design.md`. Register module in `lib.rs`.
- [ ] **T2.** Create `src/lib/paths.ts` mirroring the TS-visible constants.
- [ ] **T3.** `src-tauri/Cargo.toml`: `[lib] name = "cc_switch_lib"` → `"switchy_lib"`; update `authors`, `repository`. **Verification:** `grep -n "cc_switch\|cc-switch" src-tauri/Cargo.toml` → empty.
- [ ] **T4.** `src-tauri/src/main.rs:15`: `cc_switch_lib::run()` → `switchy_lib::run()`.
- [ ] **T5.** `package.json`: update `author`.
- [ ] **T6.** `cargo check --manifest-path src-tauri/Cargo.toml` passes.

## Phase 2 — Migration shim

- [ ] **T7.** Create `src-tauri/src/migrate_paths.rs` per design. Idempotent, logs clearly, aborts on `EXDEV`/permission errors.
- [ ] **T8.** Wire into `lib.rs::run()` as the first action before config/DB init.
- [ ] **T9.** Unit test: temp-dir scenario covers (a) only old dir exists → renamed, (b) only new dir exists → no-op, (c) both exist → warn + no-op, (d) neither → no-op.
- [ ] **T10.** `cargo test --manifest-path src-tauri/Cargo.toml migrate_paths` passes.

## Phase 3 — Replace hardcoded literals with constants

- [ ] **T11.** `src-tauri/src/config.rs:94` — use `paths::APP_DIR`. Legacy detection at `:107-108` uses `paths::LEGACY_APP_DIR` (kept intentionally).
- [ ] **T12.** `src-tauri/src/config.rs` — env var override: try `SWITCHY_HOME`, then `CC_SWITCH_HOME` (deprecation log), then default.
- [ ] **T13.** `src-tauri/src/panic_hook.rs:25` — use `paths::APP_DIR`. Update log prefix `[CC-Switch]` → `[Switchy]` at `:176`. Update test assertion `:195`.
- [ ] **T14.** `src-tauri/src/settings.rs:327` — use `paths::APP_DIR`. Update doc comment `:168`.
- [ ] **T15.** `src-tauri/src/services/env_manager.rs:71` — use `paths::APP_DIR`.
- [ ] **T16.** `src-tauri/src/lib.rs`, `database/mod.rs`, `database/backup.rs`: `"cc-switch.db"` literals → `paths::DB_FILE`. Update comments/docstrings.
- [ ] **T17.** `src-tauri/src/commands/misc.rs:918,1090,1189` — `cc_switch_launcher_*` / `cc_switch_claude_*` temp file names → `switchy_*`.
- [ ] **T18.** `src-tauri/src/mcp/opencode.rs` — comments mentioning "CC Switch" → "Switchy".
- [ ] **T19.** `src-tauri/src/proxy/http_client.rs` — "CC Switch proxy" comments + `is_cc_switch_proxy_port` helper name → `is_switchy_proxy_port`.
- [ ] **T20.** `src-tauri/src/services/provider/live.rs`, `gemini_auth.rs` — "CC-Switch"/"CC Switch" in comments/docstrings.
- [ ] **T21.** `src-tauri/src/database/backup.rs` — `validate_cc_switch_sql_export` → `validate_switchy_sql_export` (rename fn + call site at `:100`).
- [ ] **T22.** `src-tauri/src/app_config.rs:179,480,481,557` — user-visible `~/.cc-switch/config.json` mentions → `~/.switchy/config.json` (migration-friendly: the path is `~/.switchy/config.json` after migration).
- [ ] **T23.** `src-tauri/src/commands/mcp.rs:50`, `provider.rs:29`, `database/dao/skills.rs:7`, `database/migration.rs:195`, `database/schema.rs:882`, `services/skill.rs` (multiple), `commands/skill.rs:5` — doc comments referencing `~/.cc-switch/`.
- [ ] **T24.** `src/hooks/useDirectorySettings.ts:27` — use `paths::APP_DIR` from `src/lib/paths.ts`.
- [ ] **T25.** `src/types.ts:24,237`, `src/main.tsx:42` — doc comments + default path string.
- [ ] **T26.** `src/components/settings/DirectorySettings.tsx:50` — comment.
- [ ] **T27.** `src/components/providers/forms/OmoFormFields.tsx:130` — sentinel `__cc_switch_omo_variant_empty__` → `__switchy_omo_variant_empty__`.
- [ ] **T28.** `src/i18n/locales/{en,zh,ja}.json:520` — `browsePlaceholderApp` example path update (both `.cc-switch` → `.switchy`).
- [ ] **T29.** `src/components/settings/AboutSection.tsx`, `src/App.tsx`, `src/hooks/useImportExport.ts`, `src/contexts/UpdateContext.tsx`, `src/config/*ProviderPresets.ts`, `src/components/theme-provider.tsx`, `src/components/settings/WebdavSyncSection.tsx`, `src/components/providers/forms/hooks/*.ts`, `src/components/UsageScriptModal.tsx`, `src/components/AppSwitcher.tsx`, `src/lib/api/deeplink.ts` — sweep all remaining `CC Switch` / `cc-switch` occurrences; some are strings (rename), some comments (rename).
- [ ] **T30.** `src-tauri/src/deeplink/parser.rs`, `src-tauri/src/deeplink/tests.rs`, `src-tauri/src/auto_launch.rs`, `src-tauri/src/services/webdav.rs`, `src-tauri/src/services/webdav_sync.rs`, `src-tauri/src/commands/webdav_sync.rs`, `src-tauri/src/services/skill.rs`, `src-tauri/tests/**` — remaining sweep.
- [ ] **T31.** `pnpm typecheck` passes; `cargo check --manifest-path src-tauri/Cargo.toml` passes.

## Phase 4 — Flatpak

- [ ] **T32.** Rename all three `flatpak/com.ccswitch.desktop.*` files to `com.switchy.desktop.*` (git mv).
- [ ] **T33.** Edit file contents: bundle id, filesystem grants, display names, bundle artifact name.
- [ ] **T34.** `flatpak/README.md` — rewrite references.

## Phase 5 — Docs

- [ ] **T35.** `README.md` — `CC Switch` → `Switchy`; `~/.cc-switch/` → `~/.switchy/`; `cc-switch.db` → `switchy.db`.
- [ ] **T36.** `README_ZH.md`, `README_JA.md` — same, in respective languages.
- [ ] **T37.** `docs/user-manual/**` — EN + ZH + JA sweep.
- [ ] **T38.** `CONTRIBUTING.md`, `SECURITY.md`, `SUPPORT.md`, `CODE_OF_CONDUCT.md` — sweep.
- [ ] **T39.** `CHANGELOG.md` — **do not touch historical entries**; add a new top entry describing the de-fork migration.
- [ ] **T40.** `DECISION_LOG.md` — append supersession entry for 2026-04-07 namespace decision. Do not edit old entries.
- [ ] **T41.** `BACKLOG.md` — remove item 4 (namespace decision) since implemented.
- [ ] **T42.** `NEXT_SESSION.md`, `session-manager.md`, `deplink.html` — forward-looking references updated.
- [ ] **T43.** `specs/official_multi_account/{requirements,design,tasks}.md` — update path references from `~/.cc-switch/` to `~/.switchy/` (the spec is still active work).

## Phase 6 — Git remote

- [ ] **T44.** Create GitHub repo `<user-user>/switchy` (manual or `gh repo create`). Record URL.
- [ ] **T45.** `git remote set-url origin <new-url>`. Verify `git remote -v`.
- [ ] **T46.** Decide: keep `farion1231/cc-switch` as `upstream` remote, or drop entirely. User picks.

## Phase 7 — Verification

- [ ] **T47.** Full grep: `grep -rniE "cc[-_]switch|ccswitch|cc switch" --include='*.rs' --include='*.ts' --include='*.tsx' --include='*.json' --include='*.toml' --include='*.md' --include='*.yml' --include='*.xml' . | grep -v target | grep -v node_modules | grep -v SESSION_LOG | grep -v DECISION_LOG | grep -v "CHANGELOG.md:[0-9]" | grep -v "migrate_paths\|LEGACY_"` — review remaining hits; each must be justified.
- [ ] **T48.** `pnpm build` succeeds; output binary is `switchy` (not `cc-switch`).
- [ ] **T49.** Smoke test: backup `~/.cc-switch/`, run `pnpm tauri dev`, verify `~/.switchy/` created with all providers/skills intact, verify app UI shows no `CC Switch` string.
- [ ] **T50.** Commit with message `Rename Switchy: strip upstream cc-switch internal references + state-dir migration`.
