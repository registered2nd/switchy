//! History of each account's quota: how much of each rate-limit window was
//! used, as the provider reported it, kept so usage before a reset can be
//! looked up afterwards.

use crate::error::AppError;

use super::super::{lock_conn, Database};

/// Readings older than this are pruned whenever a new one is recorded.
const KEEP_DAYS: i64 = 90;

impl Database {
    /// Records one window's reading. `limit_name` is `None` for an
    /// account-wide window; times are Unix seconds.
    pub fn record_quota_reading(
        &self,
        provider_id: &str,
        limit_name: Option<&str>,
        window_minutes: i64,
        used_percent: f64,
        reset_at: Option<i64>,
        observed_at: i64,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO quota_readings
             (provider_id, limit_name, window_minutes, used_percent, reset_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                provider_id,
                limit_name,
                window_minutes,
                used_percent,
                reset_at,
                observed_at
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM quota_readings WHERE observed_at < ?1",
            [observed_at - KEEP_DAYS * 86_400],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
