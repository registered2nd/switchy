//! Plan accounts and balance monitors: a company's cards held by a key.
//!
//! A plan account (`kind` "plan") is a subscription a program reaches with
//! the plan's key; each program it is used in has a provider whose meta
//! `accountId` names it. A balance monitor (`kind` "monitor") only reads a
//! pay-as-you-go balance. Its readings are kept to tell what was spent.

use crate::error::AppError;
use serde::{Deserialize, Serialize};

use super::super::{lock_conn, Database};

/// Readings older than this are pruned whenever a new one is recorded.
const KEEP_DAYS: i64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// Catalog id of the company (`zai`, `deepseek`, ...).
    pub company: String,
    /// `plan` or `monitor`.
    pub kind: String,
    /// The company's plan or service the key belongs to.
    #[serde(default)]
    pub plan: Option<String>,
    /// `intl` or `cn`.
    #[serde(default)]
    pub region: Option<String>,
    pub name: String,
    /// The key the company issued for the plan or the balance.
    #[serde(default)]
    pub secret: String,
    /// For a Kimi plan key: the Kimi Code sign-in the key belongs to.
    #[serde(default)]
    pub login_provider_id: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub meta: serde_json::Value,
}

fn row_to_account(row: &rusqlite::Row) -> rusqlite::Result<Account> {
    let meta: String = row.get(10)?;
    Ok(Account {
        id: row.get(0)?,
        company: row.get(1)?,
        kind: row.get(2)?,
        plan: row.get(3)?,
        region: row.get(4)?,
        name: row.get(5)?,
        secret: row.get(6)?,
        login_provider_id: row.get(7)?,
        sort_index: row.get(8)?,
        created_at: row.get(9)?,
        meta: serde_json::from_str(&meta).unwrap_or(serde_json::Value::Object(Default::default())),
    })
}

const ACCOUNT_COLUMNS: &str = "id, company, kind, plan, region, name, secret, login_provider_id, sort_index, created_at, meta";

/// Inserts or replaces one account on an open connection (also used by the
/// v8 migration, which runs before the `Database` wrapper exists).
pub(crate) fn upsert_account_on_conn(
    conn: &rusqlite::Connection,
    account: &Account,
) -> Result<(), AppError> {
    let meta = if account.meta.is_null() {
        "{}".to_string()
    } else {
        account.meta.to_string()
    };
    conn.execute(
        &format!("INSERT OR REPLACE INTO accounts ({ACCOUNT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        rusqlite::params![
            account.id,
            account.company,
            account.kind,
            account.plan,
            account.region,
            account.name,
            account.secret,
            account.login_provider_id,
            account.sort_index,
            account.created_at,
            meta
        ],
    )
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// A balance reading.
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceReading {
    pub currency: String,
    pub balance: f64,
    /// Unix milliseconds.
    pub observed_at: i64,
}

impl Database {
    pub fn get_accounts(&self) -> Result<Vec<Account>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY COALESCE(sort_index, 999999), created_at"
            ))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], row_to_account)
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"
            ))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map([id], row_to_account)
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.next()
            .transpose()
            .map_err(|e| AppError::Database(e.to_string()))
    }

    pub fn save_account(&self, account: &Account) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        upsert_account_on_conn(&conn, account)
    }

    /// Deletes the account and its balance readings.
    pub fn delete_account(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("DELETE FROM balance_readings WHERE account_id = ?1", [id])
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Records a reading unless it repeats the latest one.
    pub fn record_balance_reading(
        &self,
        account_id: &str,
        reading: &BalanceReading,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let last: Option<(String, f64)> = conn
            .query_row(
                "SELECT currency, balance FROM balance_readings WHERE account_id = ?1
                 ORDER BY observed_at DESC, id DESC LIMIT 1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((currency, balance)) = last {
            if currency == reading.currency && (balance - reading.balance).abs() < 1e-9 {
                return Ok(());
            }
        }
        conn.execute(
            "INSERT INTO balance_readings (account_id, currency, balance, observed_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                account_id,
                reading.currency,
                reading.balance,
                reading.observed_at
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM balance_readings WHERE account_id = ?1 AND observed_at < ?2",
            rusqlite::params![account_id, reading.observed_at - KEEP_DAYS * 86_400_000],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// The account's readings from `since` (Unix milliseconds) on, oldest
    /// first, plus the last one before it, so a drop across `since` counts.
    pub fn get_balance_readings(
        &self,
        account_id: &str,
        since: i64,
    ) -> Result<Vec<BalanceReading>, AppError> {
        let conn = lock_conn!(self.conn);
        let map = |row: &rusqlite::Row| {
            Ok(BalanceReading {
                currency: row.get(0)?,
                balance: row.get(1)?,
                observed_at: row.get(2)?,
            })
        };
        let mut readings: Vec<BalanceReading> = conn
            .query_row(
                "SELECT currency, balance, observed_at FROM balance_readings
                 WHERE account_id = ?1 AND observed_at < ?2 ORDER BY observed_at DESC, id DESC LIMIT 1",
                rusqlite::params![account_id, since],
                map,
            )
            .ok()
            .into_iter()
            .collect();
        let mut stmt = conn
            .prepare(
                "SELECT currency, balance, observed_at FROM balance_readings
                 WHERE account_id = ?1 AND observed_at >= ?2 ORDER BY observed_at, id",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![account_id, since], map)
            .map_err(|e| AppError::Database(e.to_string()))?;
        for row in rows {
            readings.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(readings)
    }

    /// When the account's first reading was taken (Unix milliseconds).
    pub fn first_balance_reading_at(&self, account_id: &str) -> Result<Option<i64>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT MIN(observed_at) FROM balance_readings WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|e| AppError::Database(e.to_string()))
    }
}
