//! One-shot legacy-directory migration.
//!
//! Runs before any config/DB access on startup. If `~/.switchy/` is missing
//! but `~/.cc-switch/` exists (legacy install from the upstream fork era),
//! rename the directory and its DB file in place. Idempotent.
//!
//! Remove this module after one release once all users have migrated.

use std::fs;
use std::path::Path;

use crate::paths::{APP_DIR, DB_FILE, LEGACY_APP_DIR, LEGACY_DB_FILE};

/// Attempt the one-shot migration. Logs outcome, never panics.
///
/// Behavior:
/// - Neither dir exists → no-op (fresh install).
/// - New dir exists → no-op (already migrated or fresh install).
/// - Only legacy dir exists → rename to new dir, rename DB file inside.
/// - Both exist → warn, do nothing (user-resolvable conflict).
///
/// On cross-device / permission errors, logs the error and returns it so the
/// caller can decide; current call site ignores the error and lets the rest of
/// startup proceed (which will then fail naturally if state really is missing).
pub fn migrate_legacy_dir(home: &Path) -> std::io::Result<()> {
    let new_dir = home.join(APP_DIR);
    let old_dir = home.join(LEGACY_APP_DIR);

    if new_dir.exists() {
        return Ok(());
    }
    if !old_dir.exists() {
        return Ok(());
    }

    log::info!(
        "[migrate_paths] Renaming {} → {}",
        old_dir.display(),
        new_dir.display()
    );
    fs::rename(&old_dir, &new_dir)?;

    let old_db = new_dir.join(LEGACY_DB_FILE);
    let new_db = new_dir.join(DB_FILE);
    if old_db.exists() && !new_db.exists() {
        log::info!(
            "[migrate_paths] Renaming DB {} → {}",
            old_db.display(),
            new_db.display()
        );
        fs::rename(&old_db, &new_db)?;
    }

    log::info!("[migrate_paths] Migration complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn noop_when_neither_exists() {
        let tmp = tempfile::tempdir().unwrap();
        migrate_legacy_dir(tmp.path()).unwrap();
        assert!(!tmp.path().join(APP_DIR).exists());
        assert!(!tmp.path().join(LEGACY_APP_DIR).exists());
    }

    #[test]
    fn noop_when_new_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(APP_DIR)).unwrap();
        fs::create_dir_all(tmp.path().join(LEGACY_APP_DIR)).unwrap();
        migrate_legacy_dir(tmp.path()).unwrap();
        // Both still exist — no merge attempted.
        assert!(tmp.path().join(APP_DIR).exists());
        assert!(tmp.path().join(LEGACY_APP_DIR).exists());
    }

    #[test]
    fn migrates_legacy_dir_and_db() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join(LEGACY_APP_DIR);
        fs::create_dir_all(&legacy).unwrap();
        touch(&legacy.join(LEGACY_DB_FILE));
        touch(&legacy.join("settings.json"));

        migrate_legacy_dir(tmp.path()).unwrap();

        let new_dir = tmp.path().join(APP_DIR);
        assert!(new_dir.exists());
        assert!(!legacy.exists());
        assert!(new_dir.join(DB_FILE).exists());
        assert!(!new_dir.join(LEGACY_DB_FILE).exists());
        assert!(new_dir.join("settings.json").exists());
    }

    #[test]
    fn migrates_dir_even_when_db_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join(LEGACY_APP_DIR);
        fs::create_dir_all(&legacy).unwrap();

        migrate_legacy_dir(tmp.path()).unwrap();

        assert!(tmp.path().join(APP_DIR).exists());
        assert!(!legacy.exists());
    }
}
