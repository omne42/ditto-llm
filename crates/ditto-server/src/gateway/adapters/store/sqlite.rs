use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use rusqlite::OptionalExtension;
use thiserror::Error;

use super::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, ProxyRequestFingerprint,
    ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestReplayOutcome, RouterConfig, VirtualKeyConfig,
};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    path: PathBuf,
    audit_retention_secs: Option<u64>,
    audit_last_retention_reap_ms: Arc<AtomicI64>,
}

const AUDIT_RETENTION_REAP_INTERVAL_MS: i64 = 30_000;

#[derive(Debug, Error)]
pub enum SqliteStoreError {
    #[error("sqlite join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema check failed: {0}")]
    Schema(String),
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
        Self {
            path: path.into(),
            audit_retention_secs: None,
            audit_last_retention_reap_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn with_audit_retention_secs(mut self, secs: Option<u64>) -> Self {
        self.audit_retention_secs = secs.filter(|value| *value > 0);
        self
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

    pub async fn verify_schema(&self) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;

            require_sqlite_table(&conn, "virtual_keys")?;
            require_sqlite_table(&conn, "config_state")?;
            require_sqlite_table(&conn, "audit_logs")?;
            require_sqlite_table(&conn, "budget_ledger")?;
            require_sqlite_table(&conn, "budget_reservations")?;
            require_sqlite_table(&conn, "cost_ledger")?;
            require_sqlite_table(&conn, "cost_reservations")?;
            require_sqlite_table(&conn, "proxy_request_idempotency")?;

            require_sqlite_column_type(&conn, "virtual_keys", "id", "TEXT")?;
            require_sqlite_column_type(&conn, "virtual_keys", "value_json", "TEXT")?;
            require_sqlite_column_type(&conn, "config_state", "key", "TEXT")?;
            require_sqlite_column_type(&conn, "config_state", "value_json", "TEXT")?;
            require_sqlite_column_type(&conn, "audit_logs", "ts_ms", "INTEGER")?;
            require_sqlite_column_type(&conn, "audit_logs", "kind", "TEXT")?;
            require_sqlite_column_type(&conn, "audit_logs", "payload_json", "TEXT")?;
            require_sqlite_column_type(&conn, "budget_reservations", "ts_ms", "INTEGER")?;
            require_sqlite_column_type(&conn, "cost_reservations", "ts_ms", "INTEGER")?;
            require_sqlite_column_type(&conn, "proxy_request_idempotency", "state", "TEXT")?;
            require_sqlite_column_type(&conn, "proxy_request_idempotency", "record_json", "TEXT")?;
            require_sqlite_column_type(
                &conn,
                "proxy_request_idempotency",
                "expires_at_ms",
                "INTEGER",
            )?;

            require_sqlite_index(
                &conn,
                "budget_reservations",
                "idx_budget_reservations_key_id",
            )?;
            require_sqlite_index(
                &conn,
                "budget_reservations",
                "idx_budget_reservations_ts_ms",
            )?;
            require_sqlite_index(&conn, "cost_reservations", "idx_cost_reservations_key_id")?;
            require_sqlite_index(&conn, "cost_reservations", "idx_cost_reservations_ts_ms")?;
            require_sqlite_index(&conn, "audit_logs", "idx_audit_logs_ts_ms")?;
            require_sqlite_index(&conn, "audit_logs", "idx_audit_logs_kind_ts_ms")?;
            require_sqlite_index(
                &conn,
                "proxy_request_idempotency",
                "idx_proxy_request_idempotency_expires_at_ms",
            )?;
            require_sqlite_index(
                &conn,
                "proxy_request_idempotency",
                "idx_proxy_request_idempotency_state_lease_until_ms",
            )?;

            Ok(())
        })
        .await?
    }

    pub async fn ping(&self) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;
            conn.query_row("SELECT 1", [], |_| Ok(()))?;
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
            .map(|key| {
                let key = key.sanitized_for_persistence();
                Ok((key.id.clone(), serde_json::to_string(&key)?))
            })
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

    pub async fn load_router_config(&self) -> Result<Option<RouterConfig>, SqliteStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<RouterConfig>, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            let raw: Option<String> = conn
                .query_row(
                    "SELECT value_json FROM config_state WHERE key='router'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(raw) = raw else {
                return Ok(None);
            };
            Ok(Some(serde_json::from_str(&raw)?))
        })
        .await?
    }

    pub async fn replace_router_config(
        &self,
        router: &RouterConfig,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let value_json = serde_json::to_string(router)?;

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            conn.execute(
                "INSERT INTO config_state (key, value_json) VALUES ('router', ?1)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                rusqlite::params![value_json],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn replace_control_plane_snapshot(
        &self,
        keys: &[VirtualKeyConfig],
        router: &RouterConfig,
    ) -> Result<(), SqliteStoreError> {
        let path = self.path.clone();
        let serialized_keys: Vec<(String, String)> = keys
            .iter()
            .map(|key| {
                let key = key.sanitized_for_persistence();
                Ok((key.id.clone(), serde_json::to_string(&key)?))
            })
            .collect::<Result<_, serde_json::Error>>()?;
        let router_json = serde_json::to_string(router)?;

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;

            let tx = conn.transaction()?;
            tx.execute("DELETE FROM virtual_keys", [])?;
            for (id, value_json) in serialized_keys {
                tx.execute(
                    "INSERT INTO virtual_keys (id, value_json) VALUES (?1, ?2)",
                    rusqlite::params![id, value_json],
                )?;
            }
            tx.execute(
                "INSERT INTO config_state (key, value_json) VALUES ('router', ?1)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                rusqlite::params![router_json],
            )?;
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
        let commit_reserved_only = spent_tokens == u64::MAX;

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
            let committed_i64 = if commit_reserved_only {
                reserved_i64
            } else {
                spent_tokens_i64.max(0)
            };
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
        let commit_reserved_only = spent_usd_micros == u64::MAX;

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
            let committed_i64 = if commit_reserved_only {
                reserved_i64
            } else {
                spent_usd_i64.max(0)
            };
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

    pub async fn reap_stale_budget_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), SqliteStoreError> {
        let path = self.path.clone();
        let cutoff_ts_ms = tokens_to_i64(cutoff_ts_ms);
        let max_reaped = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);
        let ts_ms = now_millis();

        tokio::task::spawn_blocking(move || -> Result<(u64, u64, u64), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;

            let mut scanned = 0u64;
            let mut reaped = 0u64;
            let mut released_tokens = 0u64;

            let tx = conn.transaction()?;
            let mut stmt = tx.prepare(
                "SELECT request_id, key_id, tokens
                 FROM budget_reservations
                 WHERE ts_ms < ?1
                 ORDER BY ts_ms
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![cutoff_ts_ms, max_reaped], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            let mut reservations: Vec<(String, String, i64)> = Vec::new();
            for row in rows {
                let (request_id, key_id, tokens_i64) = row?;
                scanned = scanned.saturating_add(1);
                reaped = reaped.saturating_add(1);
                released_tokens = released_tokens.saturating_add(i64_to_u64(tokens_i64));
                reservations.push((request_id, key_id, tokens_i64));
            }
            drop(stmt);

            if !dry_run {
                for (request_id, key_id, tokens_i64) in reservations {
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
                        rusqlite::params![key_id, tokens_i64.max(0), ts_ms],
                    )?;
                }
                tx.commit()?;
            } else {
                tx.rollback()?;
            }

            Ok((scanned, reaped, released_tokens))
        })
        .await?
    }

    pub async fn reap_stale_cost_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), SqliteStoreError> {
        let path = self.path.clone();
        let cutoff_ts_ms = tokens_to_i64(cutoff_ts_ms);
        let max_reaped = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);
        let ts_ms = now_millis();

        tokio::task::spawn_blocking(move || -> Result<(u64, u64, u64), SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;

            let mut scanned = 0u64;
            let mut reaped = 0u64;
            let mut released_usd_micros = 0u64;

            let tx = conn.transaction()?;
            let mut stmt = tx.prepare(
                "SELECT request_id, key_id, usd_micros
                 FROM cost_reservations
                 WHERE ts_ms < ?1
                 ORDER BY ts_ms
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![cutoff_ts_ms, max_reaped], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            let mut reservations: Vec<(String, String, i64)> = Vec::new();
            for row in rows {
                let (request_id, key_id, usd_i64) = row?;
                scanned = scanned.saturating_add(1);
                reaped = reaped.saturating_add(1);
                released_usd_micros = released_usd_micros.saturating_add(i64_to_u64(usd_i64));
                reservations.push((request_id, key_id, usd_i64));
            }
            drop(stmt);

            if !dry_run {
                for (request_id, key_id, usd_i64) in reservations {
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
                        rusqlite::params![key_id, usd_i64.max(0), ts_ms],
                    )?;
                }
                tx.commit()?;
            } else {
                tx.rollback()?;
            }

            Ok((scanned, reaped, released_usd_micros))
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
        let retention_secs = self.audit_retention_secs;
        let should_reap =
            should_run_retention_reap(&self.audit_last_retention_reap_ms, retention_secs, ts_ms);

        tokio::task::spawn_blocking(move || -> Result<(), SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;
            conn.execute(
                "INSERT INTO audit_logs (ts_ms, kind, payload_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![ts_ms, kind, payload_json],
            )?;
            if should_reap && let Some(retention_secs) = retention_secs {
                let retention_ms = retention_secs.saturating_mul(1000);
                let retention_ms = i64::try_from(retention_ms).unwrap_or(i64::MAX);
                let cutoff_ms = ts_ms.saturating_sub(retention_ms);
                let _ = conn.execute(
                    "DELETE FROM audit_logs WHERE ts_ms < ?1",
                    rusqlite::params![cutoff_ms],
                )?;
            }
            Ok(())
        })
        .await?
    }

    pub async fn reap_audit_logs_before(&self, cutoff_ts_ms: u64) -> Result<u64, SqliteStoreError> {
        let path = self.path.clone();
        let cutoff_ts_ms = tokens_to_i64(cutoff_ts_ms);
        tokio::task::spawn_blocking(move || -> Result<u64, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;
            let deleted = conn.execute(
                "DELETE FROM audit_logs WHERE ts_ms < ?1",
                rusqlite::params![cutoff_ts_ms],
            )?;
            Ok(deleted as u64)
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

    pub async fn list_audit_logs_window(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
        before_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, SqliteStoreError> {
        let path = self.path.clone();
        let limit = i64::try_from(limit.clamp(1, 10_000)).unwrap_or(i64::MAX);
        let since_ts_ms = since_ts_ms.map(tokens_to_i64);
        let before_ts_ms = before_ts_ms.map(tokens_to_i64);

        tokio::task::spawn_blocking(move || -> Result<Vec<AuditLogRecord>, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;

            let since = since_ts_ms.unwrap_or(0);
            let before = before_ts_ms.unwrap_or(i64::MAX);

            let mut stmt = conn.prepare(
                "SELECT id, ts_ms, kind, payload_json
                 FROM audit_logs
                 WHERE ts_ms >= ?1 AND ts_ms < ?2
                 ORDER BY id DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(rusqlite::params![since, before, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;

            let mut out = Vec::new();
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

    pub async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let fingerprint = fingerprint.clone();
        let fingerprint_key = fingerprint_key.to_string();
        let owner_token = owner_token.to_string();
        let now_ms_i64 = tokens_to_i64(now_ms);
        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        let lease_until_ms_i64 = tokens_to_i64(lease_until_ms);

        tokio::task::spawn_blocking(
            move || -> Result<ProxyRequestIdempotencyBeginOutcome, SqliteStoreError> {
                let mut conn = open_connection(path)?;
                init_schema(&conn)?;
                let tx = conn.transaction()?;
                tx.execute(
                    "DELETE FROM proxy_request_idempotency WHERE expires_at_ms < ?1",
                    rusqlite::params![now_ms_i64],
                )?;

                let existing: Option<String> = tx
                    .query_row(
                        "SELECT record_json
                         FROM proxy_request_idempotency
                         WHERE request_id = ?1",
                        rusqlite::params![request_id],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(raw) = existing {
                    let record: ProxyRequestIdempotencyRecord = serde_json::from_str(&raw)?;
                    if record.fingerprint_key != fingerprint_key {
                        tx.commit()?;
                        return Ok(ProxyRequestIdempotencyBeginOutcome::Conflict { record });
                    }
                    if record.expires_at_ms >= now_ms {
                        match record.state {
                            ProxyRequestIdempotencyState::Completed => {
                                tx.commit()?;
                                return Ok(ProxyRequestIdempotencyBeginOutcome::Replay { record });
                            }
                            ProxyRequestIdempotencyState::InFlight => {
                                tx.commit()?;
                                return Ok(ProxyRequestIdempotencyBeginOutcome::InFlight {
                                    record,
                                });
                            }
                        }
                    }
                }

                let record = new_proxy_request_idempotency_record(
                    request_id.clone(),
                    fingerprint,
                    fingerprint_key,
                    owner_token.clone(),
                    now_ms,
                    lease_until_ms,
                );
                let record_json = serde_json::to_string(&record)?;
                tx.execute(
                    "INSERT INTO proxy_request_idempotency (
                         request_id,
                         state,
                         owner_token,
                         lease_until_ms,
                         expires_at_ms,
                         updated_at_ms,
                         record_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(request_id) DO UPDATE SET
                         state = excluded.state,
                         owner_token = excluded.owner_token,
                         lease_until_ms = excluded.lease_until_ms,
                         expires_at_ms = excluded.expires_at_ms,
                         updated_at_ms = excluded.updated_at_ms,
                         record_json = excluded.record_json",
                    rusqlite::params![
                        request_id,
                        proxy_request_idempotency_state_label(record.state),
                        owner_token,
                        lease_until_ms_i64,
                        lease_until_ms_i64,
                        now_ms_i64,
                        record_json,
                    ],
                )?;
                tx.commit()?;
                Ok(ProxyRequestIdempotencyBeginOutcome::Acquired)
            },
        )
        .await?
    }

    pub async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let now_ms_i64 = tokens_to_i64(now_ms);

        tokio::task::spawn_blocking(
            move || -> Result<Option<ProxyRequestIdempotencyRecord>, SqliteStoreError> {
                let conn = open_connection(path)?;
                init_schema(&conn)?;
                let raw: Option<String> = conn
                    .query_row(
                        "SELECT record_json
                         FROM proxy_request_idempotency
                         WHERE request_id = ?1 AND expires_at_ms >= ?2",
                        rusqlite::params![request_id, now_ms_i64],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(raw) = raw else {
                    return Ok(None);
                };
                Ok(Some(serde_json::from_str(&raw)?))
            },
        )
        .await?
    }

    pub async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let owner_token = owner_token.to_string();
        let now_ms_i64 = tokens_to_i64(now_ms);
        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        let lease_until_ms_i64 = tokens_to_i64(lease_until_ms);

        tokio::task::spawn_blocking(move || -> Result<bool, SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;
            let raw: Option<String> = tx
                .query_row(
                    "SELECT record_json
                     FROM proxy_request_idempotency
                     WHERE request_id = ?1",
                    rusqlite::params![request_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(raw) = raw else {
                tx.commit()?;
                return Ok(false);
            };
            let mut record: ProxyRequestIdempotencyRecord = serde_json::from_str(&raw)?;
            if record.state != ProxyRequestIdempotencyState::InFlight
                || record.owner_token.as_deref() != Some(owner_token.as_str())
            {
                tx.commit()?;
                return Ok(false);
            }

            record.updated_at_ms = now_ms;
            record.lease_until_ms = Some(lease_until_ms);
            record.expires_at_ms = lease_until_ms;
            let record_json = serde_json::to_string(&record)?;
            let updated = tx.execute(
                "UPDATE proxy_request_idempotency
                 SET lease_until_ms = ?3,
                     expires_at_ms = ?4,
                     updated_at_ms = ?5,
                     record_json = ?6
                 WHERE request_id = ?1 AND owner_token = ?2 AND state = ?7",
                rusqlite::params![
                    request_id,
                    owner_token,
                    lease_until_ms_i64,
                    lease_until_ms_i64,
                    now_ms_i64,
                    record_json,
                    proxy_request_idempotency_state_label(ProxyRequestIdempotencyState::InFlight),
                ],
            )?;
            tx.commit()?;
            Ok(updated > 0)
        })
        .await?
    }

    pub async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let owner_token = owner_token.to_string();
        let outcome = outcome.clone();
        let now_ms_i64 = tokens_to_i64(now_ms);
        let expires_at_ms = now_ms.saturating_add(replay_ttl_ms);
        let expires_at_ms_i64 = tokens_to_i64(expires_at_ms);

        tokio::task::spawn_blocking(move || -> Result<bool, SqliteStoreError> {
            let mut conn = open_connection(path)?;
            init_schema(&conn)?;
            let tx = conn.transaction()?;
            let raw: Option<String> = tx
                .query_row(
                    "SELECT record_json
                     FROM proxy_request_idempotency
                     WHERE request_id = ?1",
                    rusqlite::params![request_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(raw) = raw else {
                tx.commit()?;
                return Ok(false);
            };
            let mut record: ProxyRequestIdempotencyRecord = serde_json::from_str(&raw)?;
            if record.state != ProxyRequestIdempotencyState::InFlight
                || record.owner_token.as_deref() != Some(owner_token.as_str())
            {
                tx.commit()?;
                return Ok(false);
            }

            record.state = ProxyRequestIdempotencyState::Completed;
            record.owner_token = None;
            record.lease_until_ms = None;
            record.completed_at_ms = Some(now_ms);
            record.updated_at_ms = now_ms;
            record.expires_at_ms = expires_at_ms;
            record.outcome = Some(outcome);
            let record_json = serde_json::to_string(&record)?;
            let updated = tx.execute(
                "UPDATE proxy_request_idempotency
                 SET state = ?2,
                     owner_token = NULL,
                     lease_until_ms = NULL,
                     expires_at_ms = ?3,
                     updated_at_ms = ?4,
                     record_json = ?5
                 WHERE request_id = ?1 AND owner_token = ?6",
                rusqlite::params![
                    request_id,
                    proxy_request_idempotency_state_label(record.state),
                    expires_at_ms_i64,
                    now_ms_i64,
                    record_json,
                    owner_token,
                ],
            )?;
            tx.commit()?;
            Ok(updated > 0)
        })
        .await?
    }

    pub async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, SqliteStoreError> {
        let path = self.path.clone();
        let request_id = request_id.to_string();
        let owner_token = owner_token.to_string();

        tokio::task::spawn_blocking(move || -> Result<bool, SqliteStoreError> {
            let conn = open_connection(path)?;
            init_schema(&conn)?;
            let deleted = conn.execute(
                "DELETE FROM proxy_request_idempotency
                 WHERE request_id = ?1 AND state = ?2 AND owner_token = ?3",
                rusqlite::params![
                    request_id,
                    proxy_request_idempotency_state_label(ProxyRequestIdempotencyState::InFlight),
                    owner_token,
                ],
            )?;
            Ok(deleted > 0)
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

        CREATE TABLE IF NOT EXISTS config_state (
            key TEXT PRIMARY KEY NOT NULL,
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
        CREATE INDEX IF NOT EXISTS idx_budget_reservations_ts_ms
            ON budget_reservations(ts_ms);

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
        CREATE INDEX IF NOT EXISTS idx_cost_reservations_ts_ms
            ON cost_reservations(ts_ms);

        CREATE TABLE IF NOT EXISTS audit_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts_ms INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_logs_ts_ms
            ON audit_logs(ts_ms);
        CREATE INDEX IF NOT EXISTS idx_audit_logs_kind_ts_ms
            ON audit_logs(kind, ts_ms);

        CREATE TABLE IF NOT EXISTS proxy_request_idempotency (
            request_id TEXT PRIMARY KEY NOT NULL,
            state TEXT NOT NULL,
            owner_token TEXT,
            lease_until_ms INTEGER,
            expires_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            record_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_proxy_request_idempotency_expires_at_ms
            ON proxy_request_idempotency(expires_at_ms);
        CREATE INDEX IF NOT EXISTS idx_proxy_request_idempotency_state_lease_until_ms
            ON proxy_request_idempotency(state, lease_until_ms);",
    )?;
    Ok(())
}

fn open_connection(path: PathBuf) -> Result<rusqlite::Connection, rusqlite::Error> {
    let conn = rusqlite::Connection::open(path)?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    let _ = conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
    Ok(conn)
}

fn require_sqlite_table(conn: &rusqlite::Connection, table: &str) -> Result<(), SqliteStoreError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(SqliteStoreError::Schema(format!("missing table `{table}`")))
    }
}

fn require_sqlite_column_type(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    expected_type: &str,
) -> Result<(), SqliteStoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;

    for row in rows {
        let (name, column_type) = row?;
        if name == column {
            if column_type.eq_ignore_ascii_case(expected_type) {
                return Ok(());
            }
            return Err(SqliteStoreError::Schema(format!(
                "column `{table}.{column}` has type `{column_type}`, expected `{expected_type}`"
            )));
        }
    }
    Err(SqliteStoreError::Schema(format!(
        "missing column `{table}.{column}`"
    )))
}

fn require_sqlite_index(
    conn: &rusqlite::Connection,
    table: &str,
    index: &str,
) -> Result<(), SqliteStoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA index_list({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == index {
            return Ok(());
        }
    }
    Err(SqliteStoreError::Schema(format!(
        "missing index `{index}` on table `{table}`"
    )))
}

fn should_run_retention_reap(
    last_reap_ms: &AtomicI64,
    retention_secs: Option<u64>,
    now_ms: i64,
) -> bool {
    if retention_secs.is_none() {
        return false;
    }

    let mut observed = last_reap_ms.load(Ordering::Relaxed);
    loop {
        if observed > 0 && now_ms.saturating_sub(observed) < AUDIT_RETENTION_REAP_INTERVAL_MS {
            return false;
        }
        match last_reap_ms.compare_exchange(observed, now_ms, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => observed = actual,
        }
    }
}

fn proxy_request_idempotency_state_label(state: ProxyRequestIdempotencyState) -> &'static str {
    match state {
        ProxyRequestIdempotencyState::InFlight => "in_flight",
        ProxyRequestIdempotencyState::Completed => "completed",
    }
}

fn new_proxy_request_idempotency_record(
    request_id: String,
    fingerprint: ProxyRequestFingerprint,
    fingerprint_key: String,
    owner_token: String,
    now_ms: u64,
    lease_until_ms: u64,
) -> ProxyRequestIdempotencyRecord {
    ProxyRequestIdempotencyRecord {
        request_id,
        fingerprint,
        fingerprint_key,
        state: ProxyRequestIdempotencyState::InFlight,
        owner_token: Some(owner_token),
        started_at_ms: now_ms,
        updated_at_ms: now_ms,
        lease_until_ms: Some(lease_until_ms),
        completed_at_ms: None,
        expires_at_ms: lease_until_ms,
        outcome: None,
    }
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
mod tests;
