//! Path helpers for the Claude OAuth account snapshot store.
//!
//! Snapshots live under `~/.switchy/accounts/{provider_id}/` and hold a copy of
//! Claude Code's `.credentials.json` + the `oauthAccount` block from
//! `.claude.json`. All helpers route through `config::get_home_dir` so
//! `SWITCHY_TEST_HOME` can redirect them in tests.

use std::path::{Path, PathBuf};

use crate::config::{claude_config_json_for_dir, get_app_config_dir, get_claude_config_dir};

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

/// `~/.switchy/accounts/{provider_id}/account_state.json` — the per-account
/// root-level keys that sit *beside* `oauthAccount` (usage, entitlement, plan
/// caches). Absent for snapshots captured before this file existed.
pub fn snapshot_account_state_path(provider_id: &str) -> PathBuf {
    snapshot_dir(provider_id).join("account_state.json")
}

/// `~/.switchy/accounts/live_owner.json` — which provider's credentials we
/// last wrote into the live store. The credentials blob carries no account
/// identity of its own, so this is the only non-guessing way to attribute it
/// back to a provider on the next switch.
pub fn live_owner_path() -> PathBuf {
    get_app_config_dir()
        .join("accounts")
        .join("live_owner.json")
}

/// `~/.claude/.credentials.json` — Claude Code's on-disk credentials blob.
pub fn live_credentials_path() -> PathBuf {
    get_claude_config_dir().join(".credentials.json")
}

/// The live `.claude.json` — the file Claude Code itself reads.
///
/// It is the *sibling* of the config directory (`~/.claude` → `~/.claude.json`),
/// never a file inside it. Routing through the shared resolver keeps this
/// module and the MCP writer pointed at the same file; picking by whichever
/// candidate happens to exist silently splits them, so the identity lands in a
/// file the CLI never reads and `/status` freezes on the previous account.
pub fn live_claude_config_path() -> PathBuf {
    crate::config::get_claude_config_json_path()
}

/// Mirror-side counterpart of `live_claude_config_path`, scoped to a
/// user-configured mirror directory. Same sibling rule: a mirror pointing at
/// `\\wsl$\Ubuntu\home\me\.claude` writes `\\wsl$\Ubuntu\home\me\.claude.json`.
pub fn mirror_claude_config_path(mirror_dir: &Path) -> PathBuf {
    claude_config_json_for_dir(mirror_dir).unwrap_or_else(|| mirror_dir.join(".claude.json"))
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

    /// The decoy case that broke `/status`: a `.claude.json` sitting *inside*
    /// the config dir must never win over the sibling the CLI actually reads.
    #[test]
    #[serial]
    fn live_claude_config_path_ignores_a_file_inside_the_config_dir() {
        let home = ScopedHome::new();
        let decoy = home.path.join(".claude").join(".claude.json");
        fs::create_dir_all(decoy.parent().unwrap()).unwrap();
        fs::write(&decoy, "{}").unwrap();
        let real = home.path.join(".claude.json");
        fs::write(&real, "{}").unwrap();

        assert_eq!(live_claude_config_path(), real);
    }

    #[test]
    #[serial]
    fn live_claude_config_path_is_the_home_root_file() {
        let home = ScopedHome::new();
        // Resolution does not depend on what exists on disk.
        assert_eq!(live_claude_config_path(), home.path.join(".claude.json"));
    }

    #[test]
    #[serial]
    fn mirror_claude_config_path_is_the_sibling_of_the_mirror_dir() {
        let home = ScopedHome::new();
        let mirror = home.path.join("wsl-home").join(".claude");
        fs::create_dir_all(&mirror).unwrap();

        assert_eq!(
            mirror_claude_config_path(&mirror),
            home.path.join("wsl-home").join(".claude.json")
        );

        // A file inside the mirror dir does not change the answer.
        fs::write(mirror.join(".claude.json"), "{}").unwrap();
        assert_eq!(
            mirror_claude_config_path(&mirror),
            home.path.join("wsl-home").join(".claude.json")
        );
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
