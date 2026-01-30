use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::OptionalExtension;
use thiserror::Error;

use super::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, VirtualKeyConfig};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    path: PathBuf,
}

#[derive(Debug, Error)]
pub enum SqliteStoreError {
    #[error("sqlite join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("budget exceeded: limit={limit} attempted={attempted}")]
    BudgetExceeded { limit: u64, attempted: u64 },
    #[error(
        "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
    )]
    CostBudgetExceeded {
        limit_usd_micros: u64,
        attempted_usd_micros: u64,
    },
}

impl SqliteStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn init(&self) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;
            Ok(())
        })
        .await?
    }

    pub async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyConfig>, SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<VirtualKeyConfig>, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            let mut stmt = conn.prepare("SELECT value_json FROM virtual_keys ORDER BY id")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

            let mut keys = Vec::new();
            for row in rows {
                let raw = row?;
                let key: VirtualKeyConfig = serde_json::from_str(&raw)?;
                keys.push(key);
            }
            Ok(keys)
        })
        .await?
    }

    pub async fn replace_virtual_keys(
        &self,
        keys: &[VirtualKeyConfig],
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let serialized: Vec<(String, String)> = keys
            .iter()
            .map(|key| Ok((key.id.clone(), serde_json::to_string(key)?)))
            .collect::<Result<_, serde_json::Error>>()?;

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;

            let tx = conn.transaction()?;
            tx.execute("DELETE FROM virtual_keys", [])?;
            for (id, value_json) in serialized {
                tx.execute(
                    "INSERT INTO virtual_keys (id, value_json) VALUES (?1, ?2)",
                    rusqlite::params![id, value_json],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn reserve_budget_tokens(
        &self,
        request_id: &str,
        key_id: &str,
        limit: u64,
        tokens: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let key_id = key_id.to_string();
        let ts_ms = now_millis();
        let tokens_i64 = tokens_to_i64(tokens);
        let limit_u64 = limit;

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            tx.execute(
                "INSERT OR IGNORE INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            let (spent_tokens, reserved_tokens): (i64, i64) = tx.query_row(
                "SELECT spent_tokens, reserved_tokens FROM budget_ledger WHERE key_id=?1",
                rusqlite::params![key_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let spent_u64 = i64_to_u64(spent_tokens);
            let reserved_u64 = i64_to_u64(reserved_tokens);
            let attempted = spent_u64
                .saturating_add(reserved_u64)
                .saturating_add(tokens);
            if attempted > limit_u64 {
                return Err(SqliteStoreError::BudgetExceeded {
                    limit: limit_u64,
                    attempted,
                });
            }

            tx.execute(
                "INSERT INTO budget_reservations (request_id, key_id, tokens, ts_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![request_id, key_id, tokens_i64, ts_ms],
            )?;

            tx.execute(
                "UPDATE budget_ledger
                 SET reserved_tokens = reserved_tokens + ?2,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, tokens_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn reserve_cost_usd_micros(
        &self,
        request_id: &str,
        key_id: &str,
        limit_usd_micros: u64,
        usd_micros: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let key_id = key_id.to_string();
        let ts_ms = now_millis();
        let usd_i64 = usd_micros_to_i64(usd_micros);
        let limit_u64 = limit_usd_micros;

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            tx.execute(
                "INSERT OR IGNORE INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            let (spent_usd, reserved_usd): (i64, i64) = tx.query_row(
                "SELECT spent_usd_micros, reserved_usd_micros FROM cost_ledger WHERE key_id=?1",
                rusqlite::params![key_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let spent_u64 = i64_to_u64(spent_usd);
            let reserved_u64 = i64_to_u64(reserved_usd);
            let attempted = spent_u64
                .saturating_add(reserved_u64)
                .saturating_add(usd_micros);
            if attempted > limit_u64 {
                return Err(SqliteStoreError::CostBudgetExceeded {
                    limit_usd_micros: limit_u64,
                    attempted_usd_micros: attempted,
                });
            }

            tx.execute(
                "INSERT INTO cost_reservations (request_id, key_id, usd_micros, ts_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![request_id, key_id, usd_i64, ts_ms],
            )?;

            tx.execute(
                "UPDATE cost_ledger
                 SET reserved_usd_micros = reserved_usd_micros + ?2,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, usd_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn commit_budget_reservation_with_tokens(
        &self,
        request_id: &str,
        spent_tokens: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let ts_ms = now_millis();
        let spent_tokens_i64 = tokens_to_i64(spent_tokens);

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            let reservation: Option<(String, i64)> = tx
                .query_row(
                    "SELECT key_id, tokens FROM budget_reservations WHERE request_id=?1",
                    rusqlite::params![request_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            let Some((key_id, tokens_i64)) = reservation else {
                return Ok(());
            };

            tx.execute(
                "DELETE FROM budget_reservations WHERE request_id=?1",
                rusqlite::params![request_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            let reserved_i64 = tokens_i64.max(0);
            let committed_i64 = reserved_i64.min(spent_tokens_i64);
            tx.execute(
                "UPDATE budget_ledger
                 SET reserved_tokens = CASE WHEN reserved_tokens >= ?2 THEN reserved_tokens - ?2 ELSE 0 END,
                     spent_tokens = spent_tokens + ?3,
                     updated_at_ms = ?4
                 WHERE key_id = ?1",
                rusqlite::params![key_id, reserved_i64, committed_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn commit_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), SqliteStoreError> {
        self.commit_budget_reservation_with_tokens(request_id, u64::MAX)
            .await
    }

    pub async fn commit_cost_reservation_with_usd_micros(
        &self,
        request_id: &str,
        spent_usd_micros: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let ts_ms = now_millis();
        let spent_usd_i64 = usd_micros_to_i64(spent_usd_micros);

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            let reservation: Option<(String, i64)> = tx
                .query_row(
                    "SELECT key_id, usd_micros FROM cost_reservations WHERE request_id=?1",
                    rusqlite::params![request_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            let Some((key_id, usd_i64)) = reservation else {
                return Ok(());
            };

            tx.execute(
                "DELETE FROM cost_reservations WHERE request_id=?1",
                rusqlite::params![request_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            let reserved_i64 = usd_i64.max(0);
            let committed_i64 = reserved_i64.min(spent_usd_i64);
            tx.execute(
                "UPDATE cost_ledger
                 SET reserved_usd_micros = CASE WHEN reserved_usd_micros >= ?2 THEN reserved_usd_micros - ?2 ELSE 0 END,
                     spent_usd_micros = spent_usd_micros + ?3,
                     updated_at_ms = ?4
                 WHERE key_id = ?1",
                rusqlite::params![key_id, reserved_i64, committed_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn commit_cost_reservation(&self, request_id: &str) -> Result<(), SqliteStoreError> {
        self.commit_cost_reservation_with_usd_micros(request_id, u64::MAX)
            .await
    }

    pub async fn rollback_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let ts_ms = now_millis();

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            let reservation: Option<(String, i64)> = tx
                .query_row(
                    "SELECT key_id, tokens FROM budget_reservations WHERE request_id=?1",
                    rusqlite::params![request_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            let Some((key_id, tokens_i64)) = reservation else {
                return Ok(());
            };

            tx.execute(
                "DELETE FROM budget_reservations WHERE request_id=?1",
                rusqlite::params![request_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            tx.execute(
                "UPDATE budget_ledger
                 SET reserved_tokens = CASE WHEN reserved_tokens >= ?2 THEN reserved_tokens - ?2 ELSE 0 END,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, tokens_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn rollback_cost_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let ts_ms = now_millis();

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;

            let reservation: Option<(String, i64)> = tx
                .query_row(
                    "SELECT key_id, usd_micros FROM cost_reservations WHERE request_id=?1",
                    rusqlite::params![request_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            let Some((key_id, usd_i64)) = reservation else {
                return Ok(());
            };

            tx.execute(
                "DELETE FROM cost_reservations WHERE request_id=?1",
                rusqlite::params![request_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;

            tx.execute(
                "UPDATE cost_ledger
                 SET reserved_usd_micros = CASE WHEN reserved_usd_micros >= ?2 THEN reserved_usd_micros - ?2 ELSE 0 END,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, usd_i64, ts_ms],
            )?;

            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn record_spent_tokens(
        &self,
        key_id: &str,
        tokens: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let key_id = key_id.to_string();
        let ts_ms = now_millis();
        let tokens_i64 = tokens_to_i64(tokens);

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT OR IGNORE INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;
            tx.execute(
                "UPDATE budget_ledger
                 SET spent_tokens = spent_tokens + ?2,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, tokens_i64, ts_ms],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn record_spent_cost_usd_micros(
        &self,
        key_id: &str,
        usd_micros: u64,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let key_id = key_id.to_string();
        let ts_ms = now_millis();
        let usd_i64 = usd_micros_to_i64(usd_micros);

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT OR IGNORE INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![key_id, ts_ms],
            )?;
            tx.execute(
                "UPDATE cost_ledger
                 SET spent_usd_micros = spent_usd_micros + ?2,
                     updated_at_ms = ?3
                 WHERE key_id = ?1",
                rusqlite::params![key_id, usd_i64, ts_ms],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn append_audit_log(
        &self,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let kind = kind.into();
        let payload_json = serde_json::to_string(&payload)?;
        let ts_ms = now_millis();

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;
            conn.execute(
                "INSERT INTO audit_logs (ts_ms, kind, payload_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![ts_ms, kind, payload_json],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn list_audit_logs(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, SqliteStoreError> {
        let path = self.path.clone();
        let limit = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        tokio::task::spawn_blocking(move || -> Result<Vec<AuditLogRecord>, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            let mut out = Vec::new();
            if let Some(since) = since_ts_ms {
                let mut stmt = conn.prepare(
                    "SELECT id, ts_ms, kind, payload_json
                     FROM audit_logs
                     WHERE ts_ms >= ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![since as i64, limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (id, ts_ms, kind, payload_json) = row?;
                    let payload = serde_json::from_str(&payload_json)?;
                    out.push(AuditLogRecord {
                        id,
                        ts_ms: i64_to_u64(ts_ms),
                        kind,
                        payload,
                    });
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, ts_ms, kind, payload_json
                     FROM audit_logs
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (id, ts_ms, kind, payload_json) = row?;
                    let payload = serde_json::from_str(&payload_json)?;
                    out.push(AuditLogRecord {
                        id,
                        ts_ms: i64_to_u64(ts_ms),
                        kind,
                        payload,
                    });
                }
            }
            Ok(out)
        })
        .await?
    }

    pub async fn list_budget_ledgers(&self) -> Result<Vec<BudgetLedgerRecord>, SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(
            move || -> Result<Vec<BudgetLedgerRecord>, SqliteStoreError> {
                let conn = open_connection(path)?;
                init_schema(&conn)?;

                let mut stmt = conn.prepare(
                    "SELECT key_id, spent_tokens, reserved_tokens, updated_at_ms
                 FROM budget_ledger
                 ORDER BY key_id",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?;

                let mut out = Vec::new();
                for row in rows {
                    let (key_id, spent_tokens, reserved_tokens, updated_at_ms) = row?;
                    out.push(BudgetLedgerRecord {
                        key_id,
                        spent_tokens: i64_to_u64(spent_tokens),
                        reserved_tokens: i64_to_u64(reserved_tokens),
                        updated_at_ms: i64_to_u64(updated_at_ms),
                    });
                }
                Ok(out)
            },
        )
        .await?
    }

    pub async fn list_cost_ledgers(&self) -> Result<Vec<CostLedgerRecord>, SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<CostLedgerRecord>, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            let mut stmt = conn.prepare(
                "SELECT key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms
                     FROM cost_ledger
                     ORDER BY key_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;

            let mut out = Vec::new();
            for row in rows {
                let (key_id, spent, reserved, updated_at_ms) = row?;
                out.push(CostLedgerRecord {
                    key_id,
                    spent_usd_micros: i64_to_u64(spent),
                    reserved_usd_micros: i64_to_u64(reserved),
                    updated_at_ms: i64_to_u64(updated_at_ms),
                });
            }
            Ok(out)
        })
        .await?
    }
}

fn init_schema(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS virtual_keys (
            id TEXT PRIMARY KEY NOT NULL,
            value_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS budget_ledger (
            key_id TEXT PRIMARY KEY NOT NULL,
            spent_tokens INTEGER NOT NULL DEFAULT 0,
            reserved_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS budget_reservations (
            request_id TEXT PRIMARY KEY NOT NULL,
            key_id TEXT NOT NULL,
            tokens INTEGER NOT NULL,
            ts_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_budget_reservations_key_id
            ON budget_reservations(key_id);

        CREATE TABLE IF NOT EXISTS cost_ledger (
            key_id TEXT PRIMARY KEY NOT NULL,
            spent_usd_micros INTEGER NOT NULL DEFAULT 0,
            reserved_usd_micros INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cost_reservations (
            request_id TEXT PRIMARY KEY NOT NULL,
            key_id TEXT NOT NULL,
            usd_micros INTEGER NOT NULL,
            ts_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_cost_reservations_key_id
            ON cost_reservations(key_id);

        CREATE TABLE IF NOT EXISTS audit_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts_ms INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_logs_ts_ms
            ON audit_logs(ts_ms);
        CREATE INDEX IF NOT EXISTS idx_audit_logs_kind_ts_ms
            ON audit_logs(kind, ts_ms);",
    )?;
    Ok(())
}

fn open_connection(path: PathBuf) -> Result<rusqlite::Connection, rusqlite::Error> {
    let conn = rusqlite::Connection::open(path)?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    let _ = conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
    Ok(conn)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn tokens_to_i64(tokens: u64) -> i64 {
    if tokens > i64::MAX as u64 {
        i64::MAX
    } else {
        tokens as i64
    }
}

fn usd_micros_to_i64(usd_micros: u64) -> i64 {
    tokens_to_i64(usd_micros)
}

fn i64_to_u64(value: i64) -> u64 {
    if value <= 0 { 0 } else { value as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_store_round_trips_virtual_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        let key = VirtualKeyConfig::new("key-1", "vk-1");
        store
            .replace_virtual_keys(std::slice::from_ref(&key))
            .await
            .expect("persist");

        let loaded = store.load_virtual_keys().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "key-1");
        assert_eq!(loaded[0].token, "vk-1");

        store
            .replace_virtual_keys(&[])
            .await
            .expect("persist empty");
        let loaded = store.load_virtual_keys().await.expect("load");
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn sqlite_store_budget_reservations_enforce_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        store
            .reserve_budget_tokens("r1", "key-1", 5, 3)
            .await
            .expect("reserve r1");
        let err = store.reserve_budget_tokens("r2", "key-1", 5, 3).await;
        assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));

        store
            .rollback_budget_reservation("r1")
            .await
            .expect("rollback r1");

        store
            .reserve_budget_tokens("r3", "key-1", 5, 3)
            .await
            .expect("reserve r3");
        store
            .commit_budget_reservation("r3")
            .await
            .expect("commit r3");

        let err = store.reserve_budget_tokens("r4", "key-1", 5, 3).await;
        assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));
    }

    #[tokio::test]
    async fn sqlite_store_commit_budget_reservation_with_tokens_releases_difference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        store
            .reserve_budget_tokens("r1", "key-1", 10, 7)
            .await
            .expect("reserve r1");
        store
            .commit_budget_reservation_with_tokens("r1", 3)
            .await
            .expect("commit r1");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_tokens, 3);
        assert_eq!(ledgers[0].reserved_tokens, 0);

        store
            .reserve_budget_tokens("r2", "key-1", 10, 7)
            .await
            .expect("reserve r2");
        let err = store.reserve_budget_tokens("r3", "key-1", 10, 1).await;
        assert!(matches!(err, Err(SqliteStoreError::BudgetExceeded { .. })));

        store
            .reserve_budget_tokens("r4", "key-2", 10, 2)
            .await
            .expect("reserve r4");
        store
            .commit_budget_reservation_with_tokens("r4", 5)
            .await
            .expect("commit r4");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers 2");
        assert_eq!(ledgers.len(), 2);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[1].key_id, "key-2");
        assert_eq!(ledgers[1].spent_tokens, 2);
        assert_eq!(ledgers[1].reserved_tokens, 0);
    }

    #[tokio::test]
    async fn sqlite_store_appends_audit_logs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        store
            .append_audit_log("test", serde_json::json!({"ok": true}))
            .await
            .expect("append");

        let logs = store.list_audit_logs(10, None).await.expect("list");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].kind, "test");
        assert_eq!(logs[0].payload["ok"], true);
    }

    #[tokio::test]
    async fn sqlite_store_records_cost_ledgers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 5)
            .await
            .expect("reserve");
        store
            .commit_cost_reservation("req-1")
            .await
            .expect("commit");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_usd_micros, 5);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);
    }

    #[tokio::test]
    async fn sqlite_store_commit_cost_reservation_with_usd_micros_releases_difference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.sqlite");
        let store = SqliteStore::new(&path);
        store.init().await.expect("init");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
            .await
            .expect("reserve req-1");
        store
            .commit_cost_reservation_with_usd_micros("req-1", 3)
            .await
            .expect("commit req-1");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_usd_micros, 3);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);

        store
            .reserve_cost_usd_micros("req-2", "key-1", 10, 7)
            .await
            .expect("reserve req-2");
        let err = store.reserve_cost_usd_micros("req-3", "key-1", 10, 1).await;
        assert!(matches!(
            err,
            Err(SqliteStoreError::CostBudgetExceeded { .. })
        ));

        store
            .reserve_cost_usd_micros("req-4", "key-2", 10, 2)
            .await
            .expect("reserve req-4");
        store
            .commit_cost_reservation_with_usd_micros("req-4", 5)
            .await
            .expect("commit req-4");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers 2");
        assert_eq!(ledgers.len(), 2);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[1].key_id, "key-2");
        assert_eq!(ledgers[1].spent_usd_micros, 2);
        assert_eq!(ledgers[1].reserved_usd_micros, 0);
    }
}
