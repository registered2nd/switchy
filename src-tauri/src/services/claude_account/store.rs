//! Snapshot file I/O: atomic writes, parse-checked reads, dir deletion.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::config::atomic_write;
use crate::error::AppError;

use super::paths::snapshot_dir;

/// Atomically writes bytes to `path`, creating parent dirs as needed.
/// On Unix, sets mode `0o600` after the rename to match Claude Code's own
/// credentials-file permissions. On Windows, relies on the default per-user
/// NTFS ACL inherited from `%USERPROFILE%` (Design §Platform notes).
pub fn write_snapshot_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    atomic_write(path, bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| AppError::io(path, e))?;
    }

    Ok(())
}

/// Reads a snapshot file and parses it as JSON. The error message shape is
/// pinned by Design §Error Handling; the switch path matches on the
/// `"corrupt"` substring to produce a `credential_swap_failed:{id}` warning.
pub fn read_snapshot(path: &Path) -> Result<Value, AppError> {
    let bytes = fs::read(path).map_err(|e| AppError::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|_e| {
        AppError::Message(format!(
            "Captured snapshot is corrupt: {}",
            path.display()
        ))
    })
}

/// Removes `~/.switchy/accounts/{provider_id}/` recursively. Missing dir is
/// success — the operation is idempotent (AC-5.1).
pub fn delete_snapshot_dir(provider_id: &str) -> Result<(), AppError> {
    let dir = snapshot_dir(provider_id);
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|e| AppError::io(&dir, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct ScopedHome {
        _dir: TempDir,
        prev_test_home: Option<String>,
        prev_home: Option<String>,
        prev_userprofile: Option<String>,
    }

    impl ScopedHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let prev_test_home = env::var("SWITCHY_TEST_HOME").ok();
            let prev_home = env::var("HOME").ok();
            let prev_userprofile = env::var("USERPROFILE").ok();
            env::set_var("SWITCHY_TEST_HOME", dir.path());
            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            Self {
                _dir: dir,
                prev_test_home,
                prev_home,
                prev_userprofile,
            }
        }
    }

    impl Drop for ScopedHome {
        fn drop(&mut self) {
            match &self.prev_test_home {
                Some(v) => env::set_var("SWITCHY_TEST_HOME", v),
                None => env::remove_var("SWITCHY_TEST_HOME"),
            }
            match &self.prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match &self.prev_userprofile {
                Some(v) => env::set_var("USERPROFILE", v),
                None => env::remove_var("USERPROFILE"),
            }
        }
    }

    #[test]
    #[serial]
    fn write_then_read_round_trips_json() {
        
        let home = ScopedHome::new();
        let path = home._dir.path().join("snap.json");
        let payload = json!({ "accountUuid": "u", "emailAddress": "a@b" });
        write_snapshot_atomic(&path, serde_json::to_vec(&payload).unwrap().as_slice()).unwrap();
        let back = read_snapshot(&path).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    #[serial]
    fn read_snapshot_reports_corrupt_json() {
        
        let home = ScopedHome::new();
        let path = home._dir.path().join("corrupt.json");
        write_snapshot_atomic(&path, b"not json").unwrap();
        let err = read_snapshot(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("corrupt"),
            "expected 'corrupt' in error, got: {msg}"
        );
        assert!(msg.contains("corrupt.json"), "expected path in error, got: {msg}");
    }

    #[test]
    #[serial]
    fn delete_snapshot_dir_is_idempotent_when_missing() {
        
        let _home = ScopedHome::new();
        delete_snapshot_dir("never-existed").unwrap();
        delete_snapshot_dir("never-existed").unwrap();
    }
}
