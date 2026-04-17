//! Central constants for app directory + file names. Used by config, settings,
//! panic hook, env manager, and the one-shot legacy-directory migration.
//!
//! `LEGACY_*` exist solely to detect pre-rename installs during startup
//! migration. Remove after one release once all user data has migrated.

pub const APP_DIR: &str = ".switchy";
pub const DB_FILE: &str = "switchy.db";
pub const ENV_HOME_OVERRIDE: &str = "SWITCHY_HOME";
pub const ENV_TEST_HOME: &str = "SWITCHY_TEST_HOME";

pub const LEGACY_APP_DIR: &str = ".cc-switch";
pub const LEGACY_DB_FILE: &str = "cc-switch.db";
pub const LEGACY_ENV_HOME_OVERRIDE: &str = "CC_SWITCH_HOME";
pub const LEGACY_ENV_TEST_HOME: &str = "CC_SWITCH_TEST_HOME";
