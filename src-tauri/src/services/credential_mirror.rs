//! Background watcher that mirrors `~/.claude/.credentials.json` to the
//! configured mirror directory whenever Claude Code rotates the live file.
//!
//! Why: Claude Code rotates the OAuth refresh token on every refresh
//! (single-use, server-enforced). When both Windows and WSL hold an
//! independent copy of `.credentials.json`, whichever side refreshes first
//! invalidates the other side's refresh token. This watcher keeps the
//! mirror strictly downstream of the live file, so the mirror side
//! (typically WSL) never has to attempt its own refresh.
//!
//! Caveat: one-way live → mirror. If the mirror side's Claude Code refreshes
//! first, the live side's chain dies on its next refresh attempt.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::services::claude_account::{paths, store};

const COALESCE_WINDOW: Duration = Duration::from_millis(150);

/// Spawn the watcher on a dedicated background thread. Safe to call once at
/// app startup; no-op if no mirror dir is configured at copy time.
pub fn start() {
    let _ = std::thread::Builder::new()
        .name("cred-mirror".to_string())
        .spawn(|| {
            if let Err(e) = run() {
                log::warn!("[credential_mirror] watcher exited: {e}");
            }
        });
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let live = paths::live_credentials_path();
    let watch_dir = match live.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(()),
    };

    if !watch_dir.exists() {
        log::info!(
            "[credential_mirror] watch dir missing, skipping: {}",
            watch_dir.display()
        );
        return Ok(());
    }

    // One-shot copy so a stale mirror catches up immediately at startup.
    mirror_once(&live);

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Config::default())?;
    watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

    log::info!(
        "[credential_mirror] watching {} for changes to .credentials.json",
        watch_dir.display()
    );

    let live_filename = live.file_name().map(|s| s.to_owned());

    loop {
        let event = match rx.recv() {
            Ok(Ok(ev)) => ev,
            Ok(Err(e)) => {
                log::warn!("[credential_mirror] watch error: {e}");
                continue;
            }
            Err(_) => break, // sender dropped — watcher gone
        };

        if !is_relevant(&event, live_filename.as_deref()) {
            continue;
        }

        // Coalesce the event burst from atomic-write (write tmp + rename
        // typically produces several events back-to-back).
        std::thread::sleep(COALESCE_WINDOW);
        while rx.try_recv().is_ok() {}

        mirror_once(&live);
    }

    Ok(())
}

fn is_relevant(event: &notify::Event, live_filename: Option<&std::ffi::OsStr>) -> bool {
    if !matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
    ) {
        return false;
    }
    let Some(name) = live_filename else {
        return false;
    };
    event.paths.iter().any(|p| p.file_name() == Some(name))
}

fn mirror_once(live: &Path) {
    let Some(mirror_dir) = crate::settings::get_claude_mirror_override_dir() else {
        return;
    };
    if !live.exists() {
        return;
    }

    let target: PathBuf = mirror_dir.join(".credentials.json");

    let bytes = match std::fs::read(live) {
        Ok(b) => b,
        Err(e) => {
            log::warn!(
                "[credential_mirror] read live failed ({}): {e}",
                live.display()
            );
            return;
        }
    };

    // Skip identical content to avoid bumping mirror mtime on every event in
    // a burst.
    if let Ok(existing) = std::fs::read(&target) {
        if existing == bytes {
            return;
        }
    }

    match store::write_snapshot_atomic(&target, &bytes) {
        Ok(()) => log::info!(
            "[credential_mirror] mirrored {} bytes -> {}",
            bytes.len(),
            target.display()
        ),
        Err(e) => log::warn!(
            "[credential_mirror] write failed ({}): {e}",
            target.display()
        ),
    }
}
