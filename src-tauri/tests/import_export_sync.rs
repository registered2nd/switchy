use serde_json::json;
use std::fs;
use std::path::PathBuf;

use switchy_lib::{AppError, AppType, Provider};

#[path = "support.rs"]
mod support;
use support::{
    create_test_state, create_test_state_with_config, ensure_test_home, reset_test_fs, test_mutex,
    TestConfig,
};

#[test]
fn write_codex_live_atomic_persists_auth_and_config() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let auth = json!({ "OPENAI_API_KEY": "dev-key" });
    let config_text = r#"
[mcp_servers.echo]
type = "stdio"
command = "echo"
args = ["ok"]
"#;

    switchy_lib::write_codex_live_atomic(&auth, Some(config_text))
        .expect("atomic write should succeed");

    let auth_path = switchy_lib::get_codex_auth_path();
    let config_path = switchy_lib::get_codex_config_path();
    assert!(auth_path.exists(), "auth.json should be created");
    assert!(config_path.exists(), "config.toml should be created");

    let stored_auth: serde_json::Value =
        switchy_lib::read_json_file(&auth_path).expect("read auth");
    assert_eq!(stored_auth, auth, "auth.json should match input");

    let stored_config = std::fs::read_to_string(&config_path).expect("read config");
    assert!(
        stored_config.contains("mcp_servers.echo"),
        "config.toml should contain serialized table"
    );
}

#[test]
fn write_codex_live_atomic_rolls_back_auth_when_config_write_fails() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let auth_path = switchy_lib::get_codex_auth_path();
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).expect("create codex dir");
    }
    std::fs::write(&auth_path, r#"{"OPENAI_API_KEY":"legacy"}"#).expect("seed auth");

    let config_path = switchy_lib::get_codex_config_path();
    std::fs::create_dir_all(&config_path).expect("create blocking directory");

    let auth = json!({ "OPENAI_API_KEY": "new-key" });
    let config_text = r#"[mcp_servers.sample]
type = "stdio"
command = "noop"
"#;

    let err = switchy_lib::write_codex_live_atomic(&auth, Some(config_text))
        .expect_err("config write should fail when target is directory");
    match err {
        switchy_lib::AppError::Io { path, .. } => {
            assert!(
                path.ends_with("config.toml"),
                "io error path should point to config.toml"
            );
        }
        switchy_lib::AppError::IoContext { context, .. } => {
            assert!(
                context.contains("config.toml"),
                "error context should mention config path"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    let stored = std::fs::read_to_string(&auth_path).expect("read existing auth");
    assert!(
        stored.contains("legacy"),
        "auth.json should roll back to legacy content"
    );
    assert!(
        std::fs::metadata(&config_path)
            .expect("config path metadata")
            .is_dir(),
        "config path should remain a directory after failure"
    );
}

#[test]
fn export_sql_writes_to_target_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // Create test state with some data
    let mut config = TestConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Claude)
            .expect("claude manager");
        manager.current = "test-provider".to_string();
        manager.providers.insert(
            "test-provider".to_string(),
            Provider::with_id(
                "test-provider".to_string(),
                "Test Provider".to_string(),
                json!({"env": {"ANTHROPIC_API_KEY": "test-key"}}),
                None,
            ),
        );
    }

    let state = create_test_state_with_config(&config).expect("create test state");

    // Export to SQL file
    let export_path = home.join("test-export.sql");
    state
        .db
        .export_sql(&export_path)
        .expect("export should succeed");

    // Verify file exists and contains data
    assert!(export_path.exists(), "export file should exist");
    let content = fs::read_to_string(&export_path).expect("read exported file");
    assert!(
        content.contains("INSERT INTO") && content.contains("providers"),
        "exported SQL should contain INSERT statements for providers"
    );
    assert!(
        content.contains("test-provider"),
        "exported SQL should contain test data"
    );
}

#[test]
fn export_sql_returns_error_for_invalid_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // Try to export to an invalid path (nonexistent parent or invalid name on Windows)
    let invalid_parent = if cfg!(windows) {
        std::env::temp_dir().join("switchy-test-invalid<>dir")
    } else {
        PathBuf::from("/nonexistent/directory")
    };
    let invalid_path = invalid_parent.join("export.sql");
    let err = state
        .db
        .export_sql(&invalid_path)
        .expect_err("export to invalid path should fail");
    let invalid_prefix = invalid_parent.to_string_lossy();

    // The error can be either IoContext or Io depending on where it fails
    match err {
        AppError::IoContext { context, .. } => {
            let lc = context.to_lowercase();
            assert!(
                lc.contains("atomic") || lc.contains("write"),
                "expected IO error message about atomic write failure, got: {context}"
            );
        }
        AppError::Io { path, .. } => {
            assert!(
                path.starts_with(invalid_prefix.as_ref()),
                "expected error for {invalid_parent:?}, got: {path:?}"
            );
        }
        other => panic!("expected IoContext or Io error, got {other:?}"),
    }
}

#[test]
fn import_sql_rejects_non_cc_switch_backup() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    let import_path = home.join("not-switchy.sql");
    fs::write(&import_path, "CREATE TABLE x (id INTEGER);").expect("write import sql");

    let err = state
        .db
        .import_sql(&import_path)
        .expect_err("non-switchy sql should be rejected");

    match err {
        AppError::Localized { key, .. } => {
            assert_eq!(key, "backup.sql.invalid_format");
        }
        other => panic!("expected Localized error, got {other:?}"),
    }
}

#[test]
fn import_sql_accepts_cc_switch_exported_backup() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // Create a database with some data and export it.
    let mut config = TestConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Claude)
            .expect("claude manager");
        manager.current = "test-provider".to_string();
        manager.providers.insert(
            "test-provider".to_string(),
            Provider::with_id(
                "test-provider".to_string(),
                "Test Provider".to_string(),
                json!({"env": {"ANTHROPIC_API_KEY": "test-key"}}),
                None,
            ),
        );
    }

    let state = create_test_state_with_config(&config).expect("create test state");
    let export_path = home.join("switchy-export.sql");
    state
        .db
        .export_sql(&export_path)
        .expect("export should succeed");

    // Reset database, then import into a fresh one.
    reset_test_fs();
    let state = create_test_state().expect("create test state");
    state
        .db
        .import_sql(&export_path)
        .expect("import should succeed");

    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("load providers");
    assert!(
        providers.contains_key("test-provider"),
        "imported providers should contain test-provider"
    );
}
