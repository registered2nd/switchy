//! Schema definitions and migrations
//!
//! Creates the database tables and runs version migrations.

use super::{lock_conn, Database, SCHEMA_VERSION};
use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Serialize)]
struct LegacySkillMigrationRow {
    directory: String,
    app_type: String,
}

impl Database {
    /// Create all database tables
    pub(crate) fn create_tables(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::create_tables_on_conn(&conn)
    }

    /// Create the tables on a given connection (for migrations and tests)
    pub(crate) fn create_tables_on_conn(conn: &Connection) -> Result<(), AppError> {
        // 1. Providers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                website_url TEXT,
                category TEXT,
                created_at INTEGER,
                sort_index INTEGER,
                notes TEXT,
                icon TEXT,
                icon_color TEXT,
                meta TEXT NOT NULL DEFAULT '{}',
                is_current BOOLEAN NOT NULL DEFAULT 0,
                in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 2. Provider Endpoints table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_endpoints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                url TEXT NOT NULL,
                added_at INTEGER,
                FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 3. MCP Servers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS mcp_servers (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, server_config TEXT NOT NULL,
            description TEXT, homepage TEXT, docs TEXT, tags TEXT NOT NULL DEFAULT '[]',
            enabled_claude BOOLEAN NOT NULL DEFAULT 0, enabled_codex BOOLEAN NOT NULL DEFAULT 0,
            enabled_gemini BOOLEAN NOT NULL DEFAULT 0, enabled_opencode BOOLEAN NOT NULL DEFAULT 0,
            enabled_kimi BOOLEAN NOT NULL DEFAULT 0
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 4. Prompts table
        conn.execute("CREATE TABLE IF NOT EXISTS prompts (
            id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL, content TEXT NOT NULL,
            description TEXT, enabled BOOLEAN NOT NULL DEFAULT 1, created_at INTEGER, updated_at INTEGER,
            PRIMARY KEY (id, app_type)
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        // 5. Skills table (unified structure, v3.10.0+)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS skills (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            directory TEXT NOT NULL,
            repo_owner TEXT,
            repo_name TEXT,
            repo_branch TEXT DEFAULT 'main',
            readme_url TEXT,
            enabled_claude BOOLEAN NOT NULL DEFAULT 0,
            enabled_codex BOOLEAN NOT NULL DEFAULT 0,
            enabled_gemini BOOLEAN NOT NULL DEFAULT 0,
            enabled_opencode BOOLEAN NOT NULL DEFAULT 0,
            enabled_kimi BOOLEAN NOT NULL DEFAULT 0,
            installed_at INTEGER NOT NULL DEFAULT 0
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 6. Skill Repos table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS skill_repos (
            owner TEXT NOT NULL, name TEXT NOT NULL, branch TEXT NOT NULL DEFAULT 'main',
            enabled BOOLEAN NOT NULL DEFAULT 1, PRIMARY KEY (owner, name)
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 7. Settings table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 8. Proxy Config table (three rows, app_type primary key)
        conn.execute("CREATE TABLE IF NOT EXISTS proxy_config (
            app_type TEXT PRIMARY KEY CHECK (app_type IN ('claude','codex','gemini')),
            proxy_enabled INTEGER NOT NULL DEFAULT 0, listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 15721, enable_logging INTEGER NOT NULL DEFAULT 1,
            enabled INTEGER NOT NULL DEFAULT 0, auto_failover_enabled INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3, streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60,
            streaming_idle_timeout INTEGER NOT NULL DEFAULT 120, non_streaming_timeout INTEGER NOT NULL DEFAULT 600,
            circuit_failure_threshold INTEGER NOT NULL DEFAULT 4, circuit_success_threshold INTEGER NOT NULL DEFAULT 2,
            circuit_timeout_seconds INTEGER NOT NULL DEFAULT 60, circuit_error_rate_threshold REAL NOT NULL DEFAULT 0.6,
            circuit_min_requests INTEGER NOT NULL DEFAULT 10,
            default_cost_multiplier TEXT NOT NULL DEFAULT '1',
            pricing_model_source TEXT NOT NULL DEFAULT 'response',
            created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        // Seed the three rows (per-app defaults)
        //
        // Older databases:
        // - older proxy_config is a singleton table (no app_type column), so the three-row seed insert cannot run;
        // - the old table is converted to three rows in apply_schema_migrations() and seeded then.
        if Self::has_column(conn, "proxy_config", "app_type")? {
            conn.execute(
                "INSERT OR IGNORE INTO proxy_config (app_type, max_retries,
                streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout,
                circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                circuit_error_rate_threshold, circuit_min_requests)
                VALUES ('claude', 6, 90, 180, 600, 8, 3, 90, 0.7, 15)",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            conn.execute(
                "INSERT OR IGNORE INTO proxy_config (app_type, max_retries,
                streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout,
                circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                circuit_error_rate_threshold, circuit_min_requests)
                VALUES ('codex', 3, 60, 120, 600, 4, 2, 60, 0.6, 10)",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            conn.execute(
                "INSERT OR IGNORE INTO proxy_config (app_type, max_retries,
                streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout,
                circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                circuit_error_rate_threshold, circuit_min_requests)
                VALUES ('gemini', 5, 60, 120, 600, 4, 2, 60, 0.6, 10)",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        // 9. Provider Health table
        conn.execute("CREATE TABLE IF NOT EXISTS provider_health (
            provider_id TEXT NOT NULL, app_type TEXT NOT NULL, is_healthy INTEGER NOT NULL DEFAULT 1,
            consecutive_failures INTEGER NOT NULL DEFAULT 0, last_success_at TEXT, last_failure_at TEXT,
            last_error TEXT, updated_at TEXT NOT NULL,
            PRIMARY KEY (provider_id, app_type),
            FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        // 10. Proxy Request Logs table
        conn.execute("CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
            request_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
            duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
            provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        conn.execute("CREATE INDEX IF NOT EXISTS idx_request_logs_provider ON proxy_request_logs(provider_id, app_type)", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_request_logs_created_at ON proxy_request_logs(created_at)", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_model ON proxy_request_logs(model)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_session ON proxy_request_logs(session_id)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_status ON proxy_request_logs(status_code)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 11. Model Pricing table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 12. Stream Check Logs table
        conn.execute("CREATE TABLE IF NOT EXISTS stream_check_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT, provider_id TEXT NOT NULL, provider_name TEXT NOT NULL,
            app_type TEXT NOT NULL, status TEXT NOT NULL, success INTEGER NOT NULL, message TEXT NOT NULL,
            response_time_ms INTEGER, http_status INTEGER, model_used TEXT,
            retry_count INTEGER DEFAULT 0, tested_at INTEGER NOT NULL
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_stream_check_logs_provider
             ON stream_check_logs(app_type, provider_id, tested_at DESC)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Note: circuit_breaker_config is merged into the proxy_config table

        // 16. Proxy Live Backup table (live config backups)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS proxy_live_backup (
            app_type TEXT PRIMARY KEY, original_config TEXT NOT NULL, backed_up_at TEXT NOT NULL
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        // What the takeover last wrote to the live file; a restore merges
        // against it so other tools' edits made meanwhile survive.
        Self::add_column_if_missing(conn, "proxy_live_backup", "written_config", "TEXT")?;

        // 17. Usage Daily Rollups table (daily aggregates)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_daily_rollups (
                date TEXT NOT NULL,
                app_type TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd TEXT NOT NULL DEFAULT '0',
                avg_latency_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, app_type, provider_id, model)
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Try to add the live_takeover_active column to proxy_config
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN live_takeover_active INTEGER NOT NULL DEFAULT 0",
            [],
        );

        // Try to add the base config columns to proxy_config (for v3.9.0-2 upgrades)
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN proxy_enabled INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN listen_address TEXT NOT NULL DEFAULT '127.0.0.1'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN listen_port INTEGER NOT NULL DEFAULT 15721",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN enable_logging INTEGER NOT NULL DEFAULT 1",
            [],
        );

        // Try to add the timeout columns to proxy_config
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN streaming_idle_timeout INTEGER NOT NULL DEFAULT 120",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE proxy_config ADD COLUMN non_streaming_timeout INTEGER NOT NULL DEFAULT 600",
            [],
        );

        // If proxy_config is still the old singleton (no app_type), convert it to three rows at startup
        // At user_version=2 the v1->v2 migration no longer runs, but current queries need the app_type column.
        if Self::table_exists(conn, "proxy_config")?
            && !Self::has_column(conn, "proxy_config", "app_type")?
        {
            Self::migrate_proxy_config_to_per_app(conn)?;
        }

        // Make sure the in_failover_queue column exists (for existing v2 databases)
        Self::add_column_if_missing(
            conn,
            "providers",
            "in_failover_queue",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // Drop the old failover_queue table (if present)
        let _ = conn.execute("DROP INDEX IF EXISTS idx_failover_queue_order", []);
        let _ = conn.execute("DROP TABLE IF EXISTS failover_queue", []);

        // Index for the failover queue (on the providers table)
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_providers_failover
             ON providers(app_type, in_failover_queue, sort_index)",
            [],
        );

        Ok(())
    }

    /// Apply schema migrations
    pub(crate) fn apply_schema_migrations(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::apply_schema_migrations_on_conn(&conn)
    }

    /// Apply schema migrations on a given connection
    pub(crate) fn apply_schema_migrations_on_conn(conn: &Connection) -> Result<(), AppError> {
        conn.execute("SAVEPOINT schema_migration;", [])
            .map_err(|e| AppError::Database(format!("Failed to open migration savepoint: {e}")))?;

        let mut version = Self::get_user_version(conn)?;

        if version > SCHEMA_VERSION {
            conn.execute("ROLLBACK TO schema_migration;", []).ok();
            conn.execute("RELEASE schema_migration;", []).ok();
            return Err(AppError::Database(format!(
                "Database version is too new ({version}); this app supports up to {SCHEMA_VERSION}. Upgrade the app and try again."
            )));
        }

        let result = (|| {
            while version < SCHEMA_VERSION {
                match version {
                    0 => {
                        log::info!("user_version=0 detected, migrating to 1 (adding missing columns and setting the version)");
                        Self::migrate_v0_to_v1(conn)?;
                        Self::set_user_version(conn, 1)?;
                    }
                    1 => {
                        log::info!(
                            "Migrating database from v1 to v2 (usage tables and full columns, skills table rebuild)"
                        );
                        Self::migrate_v1_to_v2(conn)?;
                        Self::set_user_version(conn, 2)?;
                    }
                    2 => {
                        log::info!("Migrating database from v2 to v3 (unified skills management)");
                        Self::migrate_v2_to_v3(conn)?;
                        Self::set_user_version(conn, 3)?;
                    }
                    3 => {
                        log::info!("Migrating database from v3 to v4 (OpenCode support)");
                        Self::migrate_v3_to_v4(conn)?;
                        Self::set_user_version(conn, 4)?;
                    }
                    4 => {
                        log::info!("Migrating database from v4 to v5 (pricing mode support)");
                        Self::migrate_v4_to_v5(conn)?;
                        Self::set_user_version(conn, 5)?;
                    }
                    5 => {
                        log::info!("Migrating database from v5 to v6 (usage rollup table + unified Copilot template type)");
                        Self::migrate_v5_to_v6(conn)?;
                        Self::set_user_version(conn, 6)?;
                    }
                    6 => {
                        log::info!("Migrating database from v6 to v7 (Kimi Code support)");
                        Self::migrate_v6_to_v7(conn)?;
                        Self::set_user_version(conn, 7)?;
                    }
                    _ => {
                        return Err(AppError::Database(format!(
                            "Unknown database version {version}; cannot migrate to {SCHEMA_VERSION}"
                        )));
                    }
                }
                version = Self::get_user_version(conn)?;
            }
            Ok(())
        })();

        match result {
            Ok(_) => {
                conn.execute("RELEASE schema_migration;", []).map_err(|e| {
                    AppError::Database(format!("Failed to commit migration savepoint: {e}"))
                })?;
                Ok(())
            }
            Err(e) => {
                conn.execute("ROLLBACK TO schema_migration;", []).ok();
                conn.execute("RELEASE schema_migration;", []).ok();
                Err(e)
            }
        }
    }

    /// v0 -> v1 migration: add all missing columns
    fn migrate_v0_to_v1(conn: &Connection) -> Result<(), AppError> {
        // providers table
        Self::add_column_if_missing(conn, "providers", "category", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "created_at", "INTEGER")?;
        Self::add_column_if_missing(conn, "providers", "sort_index", "INTEGER")?;
        Self::add_column_if_missing(conn, "providers", "notes", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "icon", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "icon_color", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "meta", "TEXT NOT NULL DEFAULT '{}'")?;
        Self::add_column_if_missing(
            conn,
            "providers",
            "is_current",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // provider_endpoints table
        Self::add_column_if_missing(conn, "provider_endpoints", "added_at", "INTEGER")?;

        // mcp_servers table
        Self::add_column_if_missing(conn, "mcp_servers", "description", "TEXT")?;
        Self::add_column_if_missing(conn, "mcp_servers", "homepage", "TEXT")?;
        Self::add_column_if_missing(conn, "mcp_servers", "docs", "TEXT")?;
        Self::add_column_if_missing(conn, "mcp_servers", "tags", "TEXT NOT NULL DEFAULT '[]'")?;
        Self::add_column_if_missing(
            conn,
            "mcp_servers",
            "enabled_codex",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "mcp_servers",
            "enabled_gemini",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // prompts table
        Self::add_column_if_missing(conn, "prompts", "description", "TEXT")?;
        Self::add_column_if_missing(conn, "prompts", "enabled", "BOOLEAN NOT NULL DEFAULT 1")?;
        Self::add_column_if_missing(conn, "prompts", "created_at", "INTEGER")?;
        Self::add_column_if_missing(conn, "prompts", "updated_at", "INTEGER")?;

        // skills table
        Self::add_column_if_missing(conn, "skills", "installed_at", "INTEGER NOT NULL DEFAULT 0")?;

        // skill_repos table
        Self::add_column_if_missing(
            conn,
            "skill_repos",
            "branch",
            "TEXT NOT NULL DEFAULT 'main'",
        )?;
        Self::add_column_if_missing(conn, "skill_repos", "enabled", "BOOLEAN NOT NULL DEFAULT 1")?;
        // Note: the skills_path column is gone now that whole repositories are scanned recursively

        Ok(())
    }

    /// v1 -> v2 migration: add usage tables and full columns, rebuild the skills table
    fn migrate_v1_to_v2(conn: &Connection) -> Result<(), AppError> {
        // providers table columns
        Self::add_column_if_missing(
            conn,
            "providers",
            "cost_multiplier",
            "TEXT NOT NULL DEFAULT '1.0'",
        )?;
        Self::add_column_if_missing(conn, "providers", "limit_daily_usd", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "limit_monthly_usd", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "provider_type", "TEXT")?;
        Self::add_column_if_missing(
            conn,
            "providers",
            "in_failover_queue",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // Add proxy timeout columns
        if Self::table_exists(conn, "proxy_config")? {
            // Base columns missing in older versions
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "proxy_enabled",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "listen_address",
                "TEXT NOT NULL DEFAULT '127.0.0.1'",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "listen_port",
                "INTEGER NOT NULL DEFAULT 15721",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "enable_logging",
                "INTEGER NOT NULL DEFAULT 1",
            )?;

            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "streaming_first_byte_timeout",
                "INTEGER NOT NULL DEFAULT 60",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "streaming_idle_timeout",
                "INTEGER NOT NULL DEFAULT 120",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "non_streaming_timeout",
                "INTEGER NOT NULL DEFAULT 600",
            )?;
        }

        // Drop the old failover_queue table (if present)
        conn.execute("DROP INDEX IF EXISTS idx_failover_queue_order", [])
            .map_err(|e| AppError::Database(format!("Failed to drop failover_queue index: {e}")))?;
        conn.execute("DROP TABLE IF EXISTS failover_queue", [])
            .map_err(|e| AppError::Database(format!("Failed to drop failover_queue table: {e}")))?;

        // Create the failover index
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_providers_failover
             ON providers(app_type, in_failover_queue, sort_index)",
            [],
        )
        .map_err(|e| AppError::Database(format!("Failed to create failover index: {e}")))?;

        // proxy_request_logs table
        conn.execute("CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
            request_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
            duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
            provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL
        )", [])?;

        // Add new columns to the existing table
        Self::add_column_if_missing(conn, "proxy_request_logs", "provider_type", "TEXT")?;
        Self::add_column_if_missing(
            conn,
            "proxy_request_logs",
            "is_streaming",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "proxy_request_logs",
            "cost_multiplier",
            "TEXT NOT NULL DEFAULT '1.0'",
        )?;
        Self::add_column_if_missing(conn, "proxy_request_logs", "first_token_ms", "INTEGER")?;
        Self::add_column_if_missing(conn, "proxy_request_logs", "duration_ms", "INTEGER")?;

        // model_pricing table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
        )",
            [],
        )?;

        // Clear and reinsert model pricing
        conn.execute("DELETE FROM model_pricing", [])
            .map_err(|e| AppError::Database(format!("Failed to clear model pricing: {e}")))?;
        Self::seed_model_pricing(conn)?;

        // Rebuild the skills table (add the app_type column)
        Self::migrate_skills_table(conn)?;

        // Rebuild proxy_config as three rows (one config per app)
        Self::migrate_proxy_config_to_per_app(conn)?;

        Ok(())
    }

    /// Migrate proxy_config to three rows (one config per app)
    fn migrate_proxy_config_to_per_app(conn: &Connection) -> Result<(), AppError> {
        // Already the new structure? (idempotent)
        if !Self::table_exists(conn, "proxy_config")? {
            // Table missing: skip the migration (fresh install)
            return Ok(());
        }

        if Self::has_column(conn, "proxy_config", "app_type")? {
            // Already three rows: skip the migration
            log::info!("proxy_config already has three rows, skipping migration");
            return Ok(());
        }

        // Read the old config
        let old_config = conn
            .query_row(
                "SELECT listen_address, listen_port, max_retries, enable_logging,
                    streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout
             FROM proxy_config WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i32>(1)?,
                        row.get::<_, i32>(2)?,
                        row.get::<_, i32>(3)?,
                        row.get::<_, i32>(4).unwrap_or(30),
                        row.get::<_, i32>(5).unwrap_or(60),
                        row.get::<_, i32>(6).unwrap_or(300),
                    ))
                },
            )
            .unwrap_or_else(|_| ("127.0.0.1".to_string(), 5000, 3, 1, 30, 60, 300));

        let old_cb = conn.query_row(
            "SELECT failure_threshold, success_threshold, timeout_seconds, error_rate_threshold, min_requests
             FROM circuit_breaker_config WHERE id = 1", [],
            |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?, row.get::<_, i64>(2)?,
                      row.get::<_, f64>(3)?, row.get::<_, i32>(4)?))
        ).unwrap_or((5, 2, 60, 0.5, 10));

        let get_bool = |key: &str| -> bool {
            conn.query_row("SELECT value FROM settings WHERE key = ?", [key], |r| {
                r.get::<_, String>(0)
            })
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
        };

        let apps = [
            (
                "claude",
                get_bool("proxy_takeover_claude"),
                get_bool("auto_failover_enabled_claude"),
                6,
                45,
                90,
                8,
                3,
                90,
                0.6,
                15,
            ),
            (
                "codex",
                get_bool("proxy_takeover_codex"),
                get_bool("auto_failover_enabled_codex"),
                3,
                old_config.4,
                old_config.5,
                old_cb.0,
                old_cb.1,
                old_cb.2,
                old_cb.3,
                old_cb.4,
            ),
            (
                "gemini",
                get_bool("proxy_takeover_gemini"),
                get_bool("auto_failover_enabled_gemini"),
                5,
                old_config.4,
                old_config.5,
                old_cb.0,
                old_cb.1,
                old_cb.2,
                old_cb.3,
                old_cb.4,
            ),
        ];

        // Create the new table
        conn.execute("DROP TABLE IF EXISTS proxy_config_new", [])?;
        conn.execute("CREATE TABLE proxy_config_new (
            app_type TEXT PRIMARY KEY CHECK (app_type IN ('claude','codex','gemini')),
            proxy_enabled INTEGER NOT NULL DEFAULT 0, listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 15721, enable_logging INTEGER NOT NULL DEFAULT 1,
            enabled INTEGER NOT NULL DEFAULT 0, auto_failover_enabled INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3, streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60,
            streaming_idle_timeout INTEGER NOT NULL DEFAULT 120, non_streaming_timeout INTEGER NOT NULL DEFAULT 600,
            circuit_failure_threshold INTEGER NOT NULL DEFAULT 4, circuit_success_threshold INTEGER NOT NULL DEFAULT 2,
            circuit_timeout_seconds INTEGER NOT NULL DEFAULT 60, circuit_error_rate_threshold REAL NOT NULL DEFAULT 0.6,
            circuit_min_requests INTEGER NOT NULL DEFAULT 10,
            default_cost_multiplier TEXT NOT NULL DEFAULT '1',
            pricing_model_source TEXT NOT NULL DEFAULT 'response',
            created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )", [])?;

        // Insert the three rows
        for (app, takeover, failover, retries, fb, idle, cb_f, cb_s, cb_t, cb_r, cb_m) in apps {
            conn.execute(
                "INSERT INTO proxy_config_new (app_type, proxy_enabled, listen_address, listen_port, enable_logging,
                 enabled, auto_failover_enabled, max_retries, streaming_first_byte_timeout, streaming_idle_timeout,
                 non_streaming_timeout, circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                 circuit_error_rate_threshold, circuit_min_requests)
                 VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![app, old_config.0, old_config.1, old_config.3,
                    if takeover { 1 } else { 0 }, if failover { 1 } else { 0 },
                    retries, fb, idle, old_config.6, cb_f, cb_s, cb_t, cb_r, cb_m]
            ).map_err(|e| AppError::Database(format!("Failed to insert {app} config: {e}")))?;
        }

        // Swap the tables and clean up
        conn.execute("DROP TABLE IF EXISTS proxy_config", [])?;
        conn.execute("ALTER TABLE proxy_config_new RENAME TO proxy_config", [])?;
        conn.execute("DROP TABLE IF EXISTS circuit_breaker_config", [])?;
        conn.execute("DELETE FROM settings WHERE key LIKE 'proxy_takeover_%'", [])?;
        conn.execute(
            "DELETE FROM settings WHERE key LIKE 'auto_failover_enabled_%'",
            [],
        )?;

        log::info!("proxy_config migrated to three rows");
        Ok(())
    }

    /// Migrate the skills table from a single key primary key to a (directory, app_type) composite key
    fn migrate_skills_table(conn: &Connection) -> Result<(), AppError> {
        // The v3 structure (unified management) is a newer skills table:
        // - primary key is id
        // - has enabled_claude / enabled_codex / enabled_gemini and similar columns
        // In that case the v1 -> v2 migration must not run, or it fails on mismatched columns.
        if Self::has_column(conn, "skills", "enabled_claude")?
            || Self::has_column(conn, "skills", "id")?
        {
            log::info!("skills table already has the v3 structure, skipping v1 -> v2 migration");
            return Ok(());
        }

        // Already the new structure?
        if Self::has_column(conn, "skills", "app_type")? {
            log::info!("skills table already has the app_type column, skipping migration");
            return Ok(());
        }

        log::info!("Migrating skills table...");

        // 1. Rename the old table
        conn.execute("ALTER TABLE skills RENAME TO skills_old", [])
            .map_err(|e| AppError::Database(format!("Failed to rename old skills table: {e}")))?;

        // 2. Create the new table
        conn.execute(
            "CREATE TABLE skills (
                directory TEXT NOT NULL,
                app_type TEXT NOT NULL,
                installed BOOLEAN NOT NULL DEFAULT 0,
                installed_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (directory, app_type)
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("Failed to create new skills table: {e}")))?;

        // 3. Migrate data: parse the key format (e.g. "claude:my-skill" or "codex:foo")
        //    Old keys without a prefix default to claude
        let mut stmt = conn
            .prepare("SELECT key, installed, installed_at FROM skills_old")
            .map_err(|e| AppError::Database(format!("Failed to query old skills data: {e}")))?;

        let old_skills: Vec<(String, bool, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("Failed to read old skills data: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("Failed to parse old skills data: {e}")))?;

        let count = old_skills.len();

        for (key, installed, installed_at) in old_skills {
            // Parse key: "app:directory" or "directory" (defaults to claude)
            let (app_type, directory) = if let Some(idx) = key.find(':') {
                let (app, dir) = key.split_at(idx);
                (app.to_string(), dir[1..].to_string()) // skip the colon
            } else {
                ("claude".to_string(), key.clone())
            };

            conn.execute(
                "INSERT INTO skills (directory, app_type, installed, installed_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![directory, app_type, installed, installed_at],
            )
            .map_err(|e| {
                AppError::Database(format!("Failed to migrate skill {key} to the new table: {e}"))
            })?;
        }

        // 4. Drop the old table
        conn.execute("DROP TABLE skills_old", [])
            .map_err(|e| AppError::Database(format!("Failed to drop old skills table: {e}")))?;

        log::info!("skills table migration done, {count} records migrated");
        Ok(())
    }

    /// v2 -> v3 migration: unified skills management
    ///
    /// Moves the skills table from a (directory, app_type) composite key to a single id key,
    /// with per-app enabled flags (enabled_claude, enabled_codex, enabled_gemini).
    ///
    /// Strategy:
    /// 1. The old database only holds install records; the skill files live on disk
    /// 2. Rebuild the table outright; SkillService rescans the filesystem on first start to repopulate it
    fn migrate_v2_to_v3(conn: &Connection) -> Result<(), AppError> {
        // Already the new structure? (has an enabled_claude column)
        if Self::has_column(conn, "skills", "enabled_claude")? {
            log::info!("skills table already has the v3 structure, skipping migration");
            return Ok(());
        }

        log::info!("Migrating skills table to the v3 structure (unified management)...");

        // 1. Snapshot the old data (for logging and the post-startup migration)
        let old_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM skills", [], |row| row.get(0))
            .unwrap_or(0);
        log::info!("Old skills table has {old_count} records");

        let mut stmt = conn
            .prepare(
                "SELECT directory, app_type FROM skills
                 WHERE installed = 1",
            )
            .map_err(|e| AppError::Database(format!("Failed to query old skills snapshot: {e}")))?;
        let snapshot_rows: Vec<LegacySkillMigrationRow> = stmt
            .query_map([], |row| {
                Ok(LegacySkillMigrationRow {
                    directory: row.get(0)?,
                    app_type: row.get(1)?,
                })
            })
            .map_err(|e| AppError::Database(format!("Failed to read old skills snapshot: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("Failed to parse old skills snapshot: {e}")))?;
        let snapshot_json = serde_json::to_string(&snapshot_rows).map_err(|e| {
            AppError::Database(format!("Failed to serialize old skills snapshot: {e}"))
        })?;

        // Flag: rescan the filesystem after startup to rebuild the skills data
        // v3 moves the skills source of truth to ~/.switchy/skills/;
        // the old table only holds install records and cannot be migrated losslessly, so app directories are scanned and imported after startup.
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('skills_ssot_migration_pending', 'true')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('skills_ssot_migration_snapshot', ?1)",
            [snapshot_json],
        );

        // 2. Drop the old table
        conn.execute("DROP TABLE IF EXISTS skills", [])
            .map_err(|e| AppError::Database(format!("Failed to drop old skills table: {e}")))?;

        // 3. Create the new table
        conn.execute(
            "CREATE TABLE skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                directory TEXT NOT NULL,
                repo_owner TEXT,
                repo_name TEXT,
                repo_branch TEXT DEFAULT 'main',
                readme_url TEXT,
                enabled_claude BOOLEAN NOT NULL DEFAULT 0,
                enabled_codex BOOLEAN NOT NULL DEFAULT 0,
                enabled_gemini BOOLEAN NOT NULL DEFAULT 0,
                installed_at INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("Failed to create new skills table: {e}")))?;

        log::info!(
            "skills table migrated to the v3 structure.\n\
             Note: old install records were cleared; the filesystem is rescanned on first start to rebuild the data."
        );

        Ok(())
    }

    /// v3 -> v4 migration: add OpenCode support
    ///
    /// Adds the enabled_opencode column to the mcp_servers and skills tables.
    fn migrate_v3_to_v4(conn: &Connection) -> Result<(), AppError> {
        // Add enabled_opencode to mcp_servers
        Self::add_column_if_missing(
            conn,
            "mcp_servers",
            "enabled_opencode",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // Add enabled_opencode to skills
        Self::add_column_if_missing(
            conn,
            "skills",
            "enabled_opencode",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        log::info!("v3 -> v4 migration done: OpenCode support added");
        Ok(())
    }

    /// v4 -> v5 migration: add pricing mode config and the request model column
    fn migrate_v4_to_v5(conn: &Connection) -> Result<(), AppError> {
        if Self::table_exists(conn, "proxy_config")? {
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "default_cost_multiplier",
                "TEXT NOT NULL DEFAULT '1'",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "pricing_model_source",
                "TEXT NOT NULL DEFAULT 'response'",
            )?;
        }
        if Self::table_exists(conn, "proxy_request_logs")? {
            Self::add_column_if_missing(conn, "proxy_request_logs", "request_model", "TEXT")?;
        }

        log::info!("v4 -> v5 migration done: pricing mode and request model columns added");
        Ok(())
    }

    /// v6 -> v7 migration: add Kimi Code support
    ///
    /// Adds the enabled_kimi column to the mcp_servers and skills tables.
    fn migrate_v6_to_v7(conn: &Connection) -> Result<(), AppError> {
        for table in ["mcp_servers", "skills"] {
            if Self::table_exists(conn, table)? {
                Self::add_column_if_missing(
                    conn,
                    table,
                    "enabled_kimi",
                    "BOOLEAN NOT NULL DEFAULT 0",
                )?;
            }
        }
        log::info!("v6 -> v7 migration done: Kimi Code support added");
        Ok(())
    }

    /// v5 -> v6 migration: add the daily usage rollup table + unify the Copilot template type
    fn migrate_v5_to_v6(conn: &Connection) -> Result<(), AppError> {
        // 1. Add the daily usage rollup table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_daily_rollups (
                date TEXT NOT NULL,
                app_type TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd TEXT NOT NULL DEFAULT '0',
                avg_latency_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, app_type, provider_id, model)
            )",
            [],
        )
        .map_err(|e| {
            AppError::Database(format!("Failed to create usage_daily_rollups table: {e}"))
        })?;

        // 2. Unify the Copilot template type as github_copilot
        let mut stmt = conn
            .prepare("SELECT id, app_type, meta FROM providers")
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut updates = Vec::new();
        for row in rows {
            let (id, app_type, meta_str) = row.map_err(|e| AppError::Database(e.to_string()))?;

            if let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(&meta_str) {
                let mut updated = false;

                if let Some(usage_script) = meta.get_mut("usage_script") {
                    if let Some(template_type) = usage_script.get_mut("template_type") {
                        if template_type == "copilot" {
                            *template_type =
                                serde_json::Value::String("github_copilot".to_string());
                            updated = true;
                        }
                    }
                }

                if updated {
                    let new_meta_str = serde_json::to_string(&meta)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    updates.push((id, app_type, new_meta_str));
                }
            }
        }

        for (id, app_type, new_meta) in updates {
            conn.execute(
                "UPDATE providers SET meta = ?1 WHERE id = ?2 AND app_type = ?3",
                params![new_meta, id, app_type],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        log::info!("v5 -> v6 migration done: daily usage rollup table added, copilot template type unified");
        Ok(())
    }

    /// Insert the default model pricing data
    /// Format: (model_id, display_name, input, output, cache_read, cache_creation)
    /// Note: model_id uses hyphens (e.g. claude-haiku-4-5), matching normalized model names from the API
    fn seed_model_pricing(conn: &Connection) -> Result<(), AppError> {
        let pricing_data = [
            // Claude 4.6 series
            (
                "claude-opus-4-6-20260206",
                "Claude Opus 4.6",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            (
                "claude-sonnet-4-6-20260217",
                "Claude Sonnet 4.6",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // Claude 4.5 series
            (
                "claude-opus-4-5-20251101",
                "Claude Opus 4.5",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            (
                "claude-sonnet-4-5-20250929",
                "Claude Sonnet 4.5",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            (
                "claude-haiku-4-5-20251001",
                "Claude Haiku 4.5",
                "1",
                "5",
                "0.10",
                "1.25",
            ),
            // Claude 4 series (Legacy Models)
            (
                "claude-opus-4-20250514",
                "Claude Opus 4",
                "15",
                "75",
                "1.50",
                "18.75",
            ),
            (
                "claude-opus-4-1-20250805",
                "Claude Opus 4.1",
                "15",
                "75",
                "1.50",
                "18.75",
            ),
            (
                "claude-sonnet-4-20250514",
                "Claude Sonnet 4",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // Claude 3.5 series
            (
                "claude-3-5-haiku-20241022",
                "Claude 3.5 Haiku",
                "0.80",
                "4",
                "0.08",
                "1",
            ),
            (
                "claude-3-5-sonnet-20241022",
                "Claude 3.5 Sonnet",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // GPT-5.2 series
            ("gpt-5.2", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-low", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-medium", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-high", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-xhigh", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-codex", "GPT-5.2 Codex", "1.75", "14", "0.175", "0"),
            (
                "gpt-5.2-codex-low",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-medium",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-high",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-xhigh",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            // GPT-5.3 Codex series
            ("gpt-5.3-codex", "GPT-5.3 Codex", "1.75", "14", "0.175", "0"),
            (
                "gpt-5.3-codex-low",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-medium",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-high",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-xhigh",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            // GPT-5.1 series
            ("gpt-5.1", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-low", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-medium", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-high", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-minimal", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-codex", "GPT-5.1 Codex", "1.25", "10", "0.125", "0"),
            (
                "gpt-5.1-codex-mini",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max-high",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max-xhigh",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            // GPT-5 series
            ("gpt-5", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-low", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-medium", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-high", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-minimal", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-codex", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
            ("gpt-5-codex-low", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
            (
                "gpt-5-codex-medium",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-high",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini-medium",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini-high",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            // Gemini 3 series
            (
                "gemini-3-pro-preview",
                "Gemini 3 Pro Preview",
                "2",
                "12",
                "0.2",
                "0",
            ),
            (
                "gemini-3-flash-preview",
                "Gemini 3 Flash Preview",
                "0.5",
                "3",
                "0.05",
                "0",
            ),
            // Gemini 2.5 series
            (
                "gemini-2.5-pro",
                "Gemini 2.5 Pro",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gemini-2.5-flash",
                "Gemini 2.5 Flash",
                "0.3",
                "2.5",
                "0.03",
                "0",
            ),
            // StepFun series
            (
                "step-3.5-flash",
                "Step 3.5 Flash",
                "0.10",
                "0.30",
                "0.02",
                "0",
            ),
            // ====== Chinese models (CNY/1M tokens) ======
            // Doubao (ByteDance)
            (
                "doubao-seed-code",
                "Doubao Seed Code",
                "1.20",
                "8.00",
                "0.24",
                "0",
            ),
            // DeepSeek series
            (
                "deepseek-v3.2",
                "DeepSeek V3.2",
                "2.00",
                "3.00",
                "0.40",
                "0",
            ),
            (
                "deepseek-v3.1",
                "DeepSeek V3.1",
                "4.00",
                "12.00",
                "0.80",
                "0",
            ),
            ("deepseek-v3", "DeepSeek V3", "2.00", "8.00", "0.40", "0"),
            // Kimi (Moonshot AI)
            (
                "kimi-k2-thinking",
                "Kimi K2 Thinking",
                "4.00",
                "16.00",
                "1.00",
                "0",
            ),
            ("kimi-k2-0905", "Kimi K2", "4.00", "16.00", "1.00", "0"),
            (
                "kimi-k2-turbo",
                "Kimi K2 Turbo",
                "8.00",
                "58.00",
                "1.00",
                "0",
            ),
            // MiniMax series
            ("minimax-m2.1", "MiniMax M2.1", "2.10", "8.40", "0.21", "0"),
            (
                "minimax-m2.1-lightning",
                "MiniMax M2.1 Lightning",
                "2.10",
                "16.80",
                "0.21",
                "0",
            ),
            ("minimax-m2", "MiniMax M2", "2.10", "8.40", "0.21", "0"),
            // GLM (Zhipu)
            ("glm-4.7", "GLM-4.7", "2.00", "8.00", "0.40", "0"),
            ("glm-4.6", "GLM-4.6", "2.00", "8.00", "0.40", "0"),
            // Mimo (Xiaomi)
            ("mimo-v2-flash", "Mimo V2 Flash", "0", "0", "0", "0"),
        ];

        for (model_id, display_name, input, output, cache_read, cache_creation) in pricing_data {
            conn.execute(
                "INSERT OR IGNORE INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    model_id,
                    display_name,
                    input,
                    output,
                    cache_read,
                    cache_creation
                ],
            )
            .map_err(|e| AppError::Database(format!("Failed to insert model pricing: {e}")))?;
        }

        log::info!("Inserted {} default model pricing rows", pricing_data.len());
        Ok(())
    }

    /// Make sure the model pricing table has the default data
    pub fn ensure_model_pricing_seeded(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::ensure_model_pricing_seeded_on_conn(&conn)
    }

    fn ensure_model_pricing_seeded_on_conn(conn: &Connection) -> Result<(), AppError> {
        // INSERT OR IGNORE on every start: new models are added, existing rows are left alone
        Self::seed_model_pricing(conn)
    }

    // --- Helpers ---

    pub(crate) fn get_user_version(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(|e| AppError::Database(format!("Failed to read user_version: {e}")))
    }

    pub(crate) fn set_user_version(conn: &Connection, version: i32) -> Result<(), AppError> {
        if version < 0 {
            return Err(AppError::Database(
                "user_version cannot be negative".to_string(),
            ));
        }
        let sql = format!("PRAGMA user_version = {version};");
        conn.execute(&sql, [])
            .map_err(|e| AppError::Database(format!("Failed to write user_version: {e}")))?;
        Ok(())
    }

    fn validate_identifier(s: &str, kind: &str) -> Result<(), AppError> {
        if s.is_empty() {
            return Err(AppError::Database(format!("{kind} must not be empty")));
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AppError::Database(format!(
                "Invalid {kind}: {s} (only letters, digits and underscores are allowed)"
            )));
        }
        Ok(())
    }

    pub(crate) fn table_exists(conn: &Connection, table: &str) -> Result<bool, AppError> {
        Self::validate_identifier(table, "table name")?;

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .map_err(|e| AppError::Database(format!("Failed to read table names: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(format!("Failed to query table names: {e}")))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let name: String = row
                .get(0)
                .map_err(|e| AppError::Database(format!("Failed to parse table name: {e}")))?;
            if name.eq_ignore_ascii_case(table) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn has_column(
        conn: &Connection,
        table: &str,
        column: &str,
    ) -> Result<bool, AppError> {
        Self::validate_identifier(table, "table name")?;
        Self::validate_identifier(column, "column name")?;

        let sql = format!("PRAGMA table_info(\"{table}\");");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::Database(format!("Failed to read table structure: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(format!("Failed to query table structure: {e}")))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let name: String = row
                .get(1)
                .map_err(|e| AppError::Database(format!("Failed to read column name: {e}")))?;
            if name.eq_ignore_ascii_case(column) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn add_column_if_missing(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<bool, AppError> {
        Self::validate_identifier(table, "table name")?;
        Self::validate_identifier(column, "column name")?;

        if !Self::table_exists(conn, table)? {
            return Err(AppError::Database(format!(
                "Table {table} does not exist; cannot add column {column}"
            )));
        }
        if Self::has_column(conn, table, column)? {
            return Ok(false);
        }

        let sql = format!("ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {definition};");
        conn.execute(&sql, []).map_err(|e| {
            AppError::Database(format!(
                "Failed to add column {column} to table {table}: {e}"
            ))
        })?;
        log::info!("Added missing column {column} to table {table}");
        Ok(true)
    }
}
