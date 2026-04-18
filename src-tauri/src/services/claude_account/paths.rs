//! Path helpers for the Claude OAuth account snapshot store.
//!
//! Snapshots live under `~/.switchy/accounts/{provider_id}/` and hold a copy of
//! Claude Code's `.credentials.json` + the `oauthAccount` block from
//! `.claude.json`. All helpers route through `config::get_home_dir` so
//! `SWITCHY_TEST_HOME` can redirect them in tests.

use std::path::{Path, PathBuf};

use crate::config::{get_app_config_dir, get_claude_config_dir, get_home_dir};

/// `~/.switchy/accounts/{provider_id}/` — snapshot directory for a provider.
pub fn snapshot_dir(provider_id: &str) -> PathBuf {
    get_app_config_dir().join("accounts").join(provider_id)
}

/// `~/.switchy/accounts/{provider_id}/credentials.json`
pub fn snapshot_credentials_path(provider_id: &str) -> PathBuf {
    snapshot_dir(provider_id).join("credentials.json")
}

/// `~/.switchy/accounts/{provider_id}/oauth_account.json`
pub fn snapshot_oauth_account_path(provider_id: &str) -> PathBuf {
    snapshot_dir(provider_id).join("oauth_account.json")
}

/// `~/.claude/.credentials.json` — Claude Code's on-disk credentials blob.
pub fn live_credentials_path() -> PathBuf {
    get_claude_config_dir().join(".credentials.json")
}

/// Selects the live Claude config file per Design §Data Sources Row 2:
/// prefer `~/.claude/.claude.json`, else fall back to `~/.claude.json`,
/// else create the primary.
pub fn live_claude_config_path() -> PathBuf {
    let primary = get_claude_config_dir().join(".claude.json");
    if primary.exists() {
        return primary;
    }
    let fallback = get_home_dir().join(".claude.json");
    if fallback.exists() {
        return fallback;
    }
    primary
}

/// Mirror-side counterpart of `live_claude_config_path`. Same primary / legacy
/// fallback pair, scoped to a user-configured mirror directory (Design
/// §Data Sources Row 4).
pub fn mirror_claude_config_path(mirror_dir: &Path) -> PathBuf {
    let primary = mirror_dir.join(".claude.json");
    if primary.exists() {
        return primary;
    }
    let fallback = mirror_dir.join("claude.json");
    if fallback.exists() {
        return fallback;
    }
    primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use tempfile::TempDir;

    struct ScopedHome {
        _dir: TempDir,
        path: PathBuf,
        prev_test_home: Option<String>,
        prev_home: Option<String>,
        prev_userprofile: Option<String>,
    }

    impl ScopedHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let path = dir.path().to_path_buf();
            let prev_test_home = env::var("SWITCHY_TEST_HOME").ok();
            let prev_home = env::var("HOME").ok();
            let prev_userprofile = env::var("USERPROFILE").ok();
            env::set_var("SWITCHY_TEST_HOME", &path);
            env::set_var("HOME", &path);
            env::set_var("USERPROFILE", &path);
            Self {
                _dir: dir,
                path,
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
    fn snapshot_dir_nests_under_app_config_accounts() {
        
        let home = ScopedHome::new();
        let p = snapshot_dir("my-provider");
        assert_eq!(
            p,
            home.path
                .join(".switchy")
                .join("accounts")
                .join("my-provider")
        );
        assert_eq!(
            snapshot_credentials_path("my-provider"),
            p.join("credentials.json")
        );
        assert_eq!(
            snapshot_oauth_account_path("my-provider"),
            p.join("oauth_account.json")
        );
    }

    #[test]
    #[serial]
    fn live_claude_config_path_prefers_primary_when_present() {
        
        let home = ScopedHome::new();
        let primary = home.path.join(".claude").join(".claude.json");
        fs::create_dir_all(primary.parent().unwrap()).unwrap();
        fs::write(&primary, "{}").unwrap();
        // Legacy fallback also exists but primary wins.
        fs::write(home.path.join(".claude.json"), "{}").unwrap();
        assert_eq!(live_claude_config_path(), primary);
    }

    #[test]
    #[serial]
    fn live_claude_config_path_falls_back_to_legacy_home_json() {
        
        let home = ScopedHome::new();
        let legacy = home.path.join(".claude.json");
        fs::write(&legacy, "{}").unwrap();
        assert_eq!(live_claude_config_path(), legacy);
    }

    #[test]
    #[serial]
    fn live_claude_config_path_defaults_to_primary_when_neither_exists() {
        
        let home = ScopedHome::new();
        let expected = home.path.join(".claude").join(".claude.json");
        assert_eq!(live_claude_config_path(), expected);
    }

    #[test]
    #[serial]
    fn mirror_claude_config_path_primary_fallback_create() {
        
        let home = ScopedHome::new();
        let mirror = home.path.join("mirror");
        fs::create_dir_all(&mirror).unwrap();

        // Neither exists → returns primary.
        assert_eq!(
            mirror_claude_config_path(&mirror),
            mirror.join(".claude.json")
        );

        // Legacy exists → returns legacy.
        let legacy = mirror.join("claude.json");
        fs::write(&legacy, "{}").unwrap();
        assert_eq!(mirror_claude_config_path(&mirror), legacy);

        // Primary exists → takes precedence.
        let primary = mirror.join(".claude.json");
        fs::write(&primary, "{}").unwrap();
        assert_eq!(mirror_claude_config_path(&mirror), primary);
    }

    #[test]
    #[serial]
    fn live_credentials_path_sits_under_claude_dir() {
        
        let home = ScopedHome::new();
        assert_eq!(
            live_credentials_path(),
            home.path.join(".claude").join(".credentials.json")
        );
    }
}
