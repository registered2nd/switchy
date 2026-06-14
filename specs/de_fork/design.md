# De-fork Switchy — Design

## Centralize the dir-name constant
Today, `".cc-switch"` is hardcoded in six places:
- `src-tauri/src/config.rs:94`, `:107-108` (legacy `HOME` fallback)
- `src-tauri/src/panic_hook.rs:25`
- `src-tauri/src/settings.rs:327`
- `src-tauri/src/services/env_manager.rs:71`
- `src/hooks/useDirectorySettings.ts:27`

And `"cc-switch.db"` in ~4 places. Rather than touch each call site and risk drift later, introduce a single module `src-tauri/src/paths.rs` that exports:

```rust
pub const APP_DIR: &str = ".switchy";
pub const LEGACY_APP_DIR: &str = ".cc-switch";
pub const DB_FILE: &str = "switchy.db";
pub const LEGACY_DB_FILE: &str = "cc-switch.db";
pub const ENV_HOME_OVERRIDE: &str = "SWITCHY_HOME";
pub const LEGACY_ENV_HOME_OVERRIDE: &str = "CC_SWITCH_HOME";
```

All call sites read from this module. TS side: mirror the constant in `src/lib/paths.ts` exporting the same strings; `useDirectorySettings.ts` uses it.

## Migration shim (`src-tauri/src/migrate_paths.rs`)
Runs once on startup before any DB or config read. Pseudocode:

```
let home = get_home_dir();
let new_dir = home.join(APP_DIR);
let old_dir = home.join(LEGACY_APP_DIR);

if !new_dir.exists() && old_dir.exists() {
    log::info!("Migrating ~/{LEGACY_APP_DIR} → ~/{APP_DIR}");
    std::fs::rename(&old_dir, &new_dir)?;

    let old_db = new_dir.join(LEGACY_DB_FILE);
    let new_db = new_dir.join(DB_FILE);
    if old_db.exists() && !new_db.exists() {
        std::fs::rename(&old_db, &new_db)?;
    }
    log::info!("Migration complete");
}
```

Idempotent: if both dirs exist (user manually created one), skip and log a warning — do NOT merge. If rename fails (permission, cross-device link on Linux if `~/` is a mount point), log the error and abort startup with a clear message. Call site: `lib.rs::run()`, very first line.

**Legacy env var.** `SWITCHY_HOME` wins. If unset, fall back to `CC_SWITCH_HOME` with a deprecation log. Drop after one release.

## Build identity changes
| File | Change |
|---|---|
| `src-tauri/Cargo.toml` | `[lib] name = "switchy_lib"`; `authors = ["the user Shu Zhang"]`; `repository = "https://github.com/<TBD>/switchy"` |
| `src-tauri/src/main.rs:15` | `switchy_lib::run()` |
| `package.json` | `author: "the user Shu Zhang"` |
| `src-tauri/tauri.conf.json` | already `Switchy` / `com.switchy.desktop` / `switchy` scheme — verify only |

## Flatpak
Rename files:
- `flatpak/com.ccswitch.desktop.yml` → `com.switchy.desktop.yml`
- `flatpak/com.ccswitch.desktop.metainfo.xml` → `com.switchy.desktop.metainfo.xml`
- `flatpak/com.ccswitch.desktop.desktop` → `com.switchy.desktop.desktop`

Edit contents: every `com.ccswitch.desktop` string → `com.switchy.desktop`. `~/.cc-switch` filesystem grants → `~/.switchy`. `CC-Switch-Linux.flatpak` → `Switchy-Linux.flatpak`. `for CC Switch` prose → `for Switchy`.

## Launcher temp files
`src-tauri/src/commands/misc.rs` uses `cc_switch_launcher_{pid}.sh` / `cc_switch_claude_{pid}.bat`. Rename to `switchy_launcher_*.sh` / `switchy_claude_*.bat`. No backward compat needed — these are ephemeral temp files.

## Deep-link plugin path
`lib.rs:588` comment mentions `~/.local/share/com.ccswitch.desktop/applications/cc-switch-handler.desktop`. That path is computed by `tauri-plugin-deep-link` from the bundle identifier, which is already `com.switchy.desktop` — so the actual path becomes `~/.local/share/com.switchy.desktop/applications/switchy-handler.desktop`. Update the comment only.

## Docs strategy
**User-facing docs:** rewrite. `CC Switch` / `CC-Switch` → `Switchy`. `~/.cc-switch/` → `~/.switchy/`. `cc-switch.db` → `switchy.db`.

**CHANGELOG.md historical entries:** preserve. Those describe shipped versions that literally wrote to `~/.cc-switch/`. Add a new entry at the top for the de-fork release that documents the migration.

**DECISION_LOG.md:** append a new dated entry: *"Supersedes 2026-04-07 decision on data namespace. Switchy is no longer a fork; `~/.cc-switch/` migrated to `~/.switchy/` via one-shot shim. Old entries preserved verbatim."* Do not edit the old entries.

**BACKLOG.md:** remove item 4 ("Decide on inherited upstream data namespace") since it's now decided and implemented.

## Git remote
Current: `origin → farion1231/cc-switch.git`. Two steps:
1. Create new GitHub repo under the user's account: `<user>/switchy`. Manual step or `gh repo create`.
2. `git remote set-url origin https://github.com/<user>/switchy.git`. Optionally keep upstream as a named remote `upstream` if still want to cherry-pick — decide at task time.

## Verification
- Full-tree grep for `cc-switch`, `ccswitch`, `cc_switch`, `CC Switch`, `CC-Switch` returns only: migration shim, `DECISION_LOG.md` history, `SESSION_LOG.md` history, `CHANGELOG.md` historical entries.
- `pnpm typecheck` passes.
- `cargo check --manifest-path src-tauri/Cargo.toml` passes.
- `pnpm test:unit` and `cargo test --manifest-path src-tauri/Cargo.toml` pass.
- Manual smoke: launch with existing `~/.cc-switch/` populated → confirm `~/.switchy/` appears and app shows all providers.

## Risks
- **Cross-device rename.** If `$HOME` is a mount point different from where `~/.cc-switch/` actually lives (symlinks, WSL bind mounts), `fs::rename` fails with `EXDEV`. Fall back to copy-and-delete in that case, or abort with clear error. Pick: **abort with error** — safer, rare case.
- **Stale tauri-plugin-deep-link state.** The plugin writes `.desktop` files under the old `com.ccswitch.desktop/` dir. After identifier change, the old desktop file is orphaned. Document this in the migration log; plugin will recreate under new ID.
- **Windows single-instance lock.** If `tauri-plugin-single-instance` keys off bundle ID, a running old-build instance can't conflict (different IDs). Fine.
