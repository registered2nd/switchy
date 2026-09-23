//! Database module - SQLite persistence
//!
//! Core data storage for the app, covering:
//! - Provider configuration
//! - MCP server configuration
//! - Prompts
//! - Skills
//! - General settings
//!
//! ## Architecture
//!
//! ```text
//! database/
//! ├── mod.rs        - Database struct + initialization
//! ├── schema.rs     - Table definitions + schema migrations
//! ├── backup.rs     - SQL import/export + snapshot backups
//! ├── migration.rs  - JSON -> SQLite data migration
//! └── dao/          - Data access objects
//!     ├── providers.rs
//!     ├── mcp.rs
//!     ├── prompts.rs
//!     ├── skills.rs
//!     └── settings.rs
//! ```

pub(crate) mod backup;
mod dao;
mod schema;

#[cfg(test)]
mod tests;

// DAO types exported for external use
pub use dao::FailoverQueueItem;

use crate::config::get_app_config_dir;
use crate::error::AppError;
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;

// DAO methods live on impl Database, so nothing else needs exporting

/// Current schema version
/// Bump on every table structure change and add the matching migration in schema.rs
pub(crate) const SCHEMA_VERSION: i32 = 7;

/// Serialize JSON without an unwrap panic
pub(crate) fn to_json_string<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|e| AppError::Config(format!("JSON serialization failed: {e}")))
}

/// Lock the Mutex without an unwrap panic
macro_rules! lock_conn {
    ($mutex:expr) => {
        $mutex
            .lock()
            .map_err(|e| AppError::Database(format!("Mutex lock failed: {}", e)))?
    };
}

// Export macros for submodules
pub(crate) use lock_conn;

/// Database connection wrapper
///
/// Wraps the Connection in a Mutex so it can be shared across threads (e.g. Tauri State).
/// rusqlite::Connection is not Sync on its own, hence the wrapper.
pub struct Database {
    pub(crate) conn: Mutex<Connection>,
}

impl Database {
    /// Open the database connection and create the tables
    ///
    /// The database file is `~/.switchy/switchy.db`
    pub fn init() -> Result<Self, AppError> {
        let db_path = get_app_config_dir().join(crate::paths::DB_FILE);
        let db_exists = db_path.exists();

        // Make sure the parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        let conn = Connection::open(&db_path).map_err(|e| AppError::Database(e.to_string()))?;

        // Enable foreign key constraints
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        if !db_exists {
            // For a brand-new database, configure incremental auto-vacuum
            // before creating any tables so no rebuild is needed later.
            conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_tables()?;

        // Pre-migration backup: only when upgrading from an existing database
        {
            let conn = lock_conn!(db.conn);
            let version = Self::get_user_version(&conn)?;
            drop(conn);
            if version > 0 && version < SCHEMA_VERSION {
                log::info!(
                    "Creating pre-migration database backup (v{version} → v{SCHEMA_VERSION})"
                );
                if let Err(e) = db.backup_database_file() {
                    log::warn!("Pre-migration backup failed, continuing migration: {e}");
                }
            }
        }

        db.apply_schema_migrations()?;
        if let Err(e) = db.ensure_incremental_auto_vacuum() {
            log::warn!("Failed to ensure incremental auto-vacuum: {e}");
        }
        db.ensure_model_pricing_seeded()?;

        // Startup cleanup: prune old logs and reclaim space
        if let Err(e) = db.cleanup_old_stream_check_logs(7) {
            log::warn!("Startup stream_check_logs cleanup failed: {e}");
        }
        if let Err(e) = db.rollup_and_prune(30) {
            log::warn!("Startup rollup_and_prune failed: {e}");
        }
        // Reclaim disk space after cleanup
        {
            let conn = lock_conn!(db.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Startup incremental vacuum failed: {e}");
            }
        }

        Ok(db)
    }

    /// Create an in-memory database (for tests)
    pub fn memory() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        // Enable foreign key constraints
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_tables()?;
        db.ensure_model_pricing_seeded()?;

        Ok(db)
    }

    pub(crate) fn get_auto_vacuum_mode(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA auto_vacuum;", [], |row| row.get(0))
            .map_err(|e| AppError::Database(format!("Failed to read auto_vacuum: {e}")))
    }

    fn has_user_tables(conn: &Connection) -> Result<bool, AppError> {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(format!("Failed to read table count: {e}")))?;
        Ok(count > 0)
    }

    pub(crate) fn ensure_incremental_auto_vacuum_on_conn(
        conn: &Connection,
    ) -> Result<bool, AppError> {
        let mode = Self::get_auto_vacuum_mode(conn)?;
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = Self::has_user_tables(conn)?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(format!("Failed to set auto_vacuum: {e}")))?;

        if !has_tables {
            return Ok(false);
        }

        conn.execute("VACUUM;", [])
            .map_err(|e| AppError::Database(format!("VACUUM failed: {e}")))?;
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(format!("Failed to restore foreign_keys: {e}")))?;
        Ok(true)
    }

    pub(crate) fn ensure_incremental_auto_vacuum(&self) -> Result<bool, AppError> {
        let mode = {
            let conn = lock_conn!(self.conn);
            Self::get_auto_vacuum_mode(&conn)?
        };
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = {
            let conn = lock_conn!(self.conn);
            Self::has_user_tables(&conn)?
        };
        if has_tables {
            log::info!(
                "Detected auto_vacuum={mode}, rebuilding database to enable incremental vacuum"
            );
            self.backup_database_file()?;
        }

        let rebuilt = {
            let conn = lock_conn!(self.conn);
            Self::ensure_incremental_auto_vacuum_on_conn(&conn)?
        };

        if rebuilt {
            log::info!("Incremental auto-vacuum enabled after database rebuild");
        } else {
            log::info!("Incremental auto-vacuum configured for new database");
        }

        Ok(rebuilt)
    }

    /// Check whether the MCP servers table is empty
    pub fn is_mcp_table_empty(&self) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count == 0)
    }

    /// Check whether the prompts table is empty
    pub fn is_prompts_table_empty(&self) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count == 0)
    }
}
