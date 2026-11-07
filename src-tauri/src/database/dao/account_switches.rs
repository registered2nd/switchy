//! History of account switches: which account took over an app, from which,
//! and why.

use crate::error::AppError;
use serde::{Deserialize, Serialize};

use super::super::{lock_conn, Database};

/// Switches older than this are pruned whenever a new one is recorded.
const KEEP_DAYS: i64 = 90;

/// Why the account serving an app changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchReason {
    /// Someone picked the account (window, tray).
    Manual,
    /// The previous account failed a request and the next one answered.
    Failover,
    /// The previous account hit its usage limit.
    Limit,
    /// The previous account's login was refused; it needs signing in again.
    SignedOut,
    /// The pool served the request from another account first (the current
    /// one was near its limit or its circuit breaker was open).
    Rotation,
    /// An account earlier in the switching order recovered and took over again.
    Recovered,
}

impl SwitchReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Failover => "failover",
            Self::Limit => "limit",
            Self::SignedOut => "signed_out",
            Self::Rotation => "rotation",
            Self::Recovered => "recovered",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "failover" => Self::Failover,
            "limit" => Self::Limit,
            "signed_out" => Self::SignedOut,
            "rotation" => Self::Rotation,
            "recovered" => Self::Recovered,
            _ => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSwitch {
    pub id: i64,
    pub app_type: String,
    pub from_provider_id: Option<String>,
    pub from_provider_name: Option<String>,
    pub from_account: Option<String>,
    pub to_provider_id: String,
    pub to_provider_name: Option<String>,
    pub to_account: Option<String>,
    pub reason: SwitchReason,
    pub detail: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
}

impl Database {
    pub fn record_account_switch(
        &self,
        app_type: &str,
        from_provider_id: Option<&str>,
        to_provider_id: &str,
        reason: SwitchReason,
        detail: Option<&str>,
    ) -> Result<(), AppError> {
        if from_provider_id == Some(to_provider_id) {
            return Ok(());
        }
        let now = chrono::Utc::now().timestamp();
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO account_switches
             (app_type, from_provider_id, to_provider_id, reason, detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                app_type,
                from_provider_id,
                to_provider_id,
                reason.as_str(),
                detail,
                now
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM account_switches WHERE created_at < ?1",
            [now - KEEP_DAYS * 86_400],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Newest first, optionally for one app and from a start time (Unix seconds).
    pub fn get_account_switches(
        &self,
        app_type: Option<&str>,
        since: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AccountSwitch>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.app_type, s.from_provider_id, pf.name, s.to_provider_id, pt.name,
                        s.reason, s.detail, s.created_at
                 FROM account_switches s
                 LEFT JOIN providers pf ON pf.id = s.from_provider_id AND pf.app_type = s.app_type
                 LEFT JOIN providers pt ON pt.id = s.to_provider_id AND pt.app_type = s.app_type
                 WHERE (?1 IS NULL OR s.app_type = ?1) AND (?2 IS NULL OR s.created_at >= ?2)
                 ORDER BY s.created_at DESC, s.id DESC
                 LIMIT ?3",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![app_type, since, limit], |row| {
                Ok(AccountSwitch {
                    id: row.get(0)?,
                    app_type: row.get(1)?,
                    from_provider_id: row.get(2)?,
                    from_provider_name: row.get(3)?,
                    from_account: None,
                    to_provider_id: row.get(4)?,
                    to_provider_name: row.get(5)?,
                    to_account: None,
                    reason: SwitchReason::parse(&row.get::<_, String>(6)?),
                    detail: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut switches = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);
        drop(conn);

        // Every pooled card of an app shares a name, so name the accounts.
        let mut providers_by_app = std::collections::HashMap::new();
        for switch in &mut switches {
            let providers = providers_by_app
                .entry(switch.app_type.clone())
                .or_insert_with(|| self.get_all_providers(&switch.app_type).unwrap_or_default());
            switch.from_account = switch
                .from_provider_id
                .as_ref()
                .and_then(|id| providers.get(id))
                .and_then(|p| p.account_email());
            switch.to_account = providers
                .get(&switch.to_provider_id)
                .and_then(|p| p.account_email());
        }
        Ok(switches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_come_back_newest_first_and_a_non_switch_is_not_recorded() {
        let db = Database::memory().unwrap();
        db.record_account_switch("codex", Some("a"), "b", SwitchReason::Limit, Some("429"))
            .unwrap();
        db.record_account_switch("codex", Some("b"), "b", SwitchReason::Manual, None)
            .unwrap();
        db.record_account_switch("claude", None, "c", SwitchReason::Manual, None)
            .unwrap();

        let codex = db.get_account_switches(Some("codex"), None, 50).unwrap();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].reason, SwitchReason::Limit);
        assert_eq!(codex[0].detail.as_deref(), Some("429"));

        let all = db.get_account_switches(None, None, 50).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].app_type, "claude");
    }
}
