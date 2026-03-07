use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use sqlx::Row;
use sqlx::mysql::MySqlPoolOptions;
use thiserror::Error;

use super::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, RouterConfig, VirtualKeyConfig};

#[derive(Clone, Debug)]
pub struct MySqlStore {
    pool: sqlx::MySqlPool,
    audit_retention_secs: Option<u64>,
    audit_last_retention_reap_ms: Arc<AtomicI64>,
}

const AUDIT_RETENTION_REAP_INTERVAL_MS: i64 = 30_000;

#[derive(Debug, Error)]
pub enum MySqlStoreError {
    #[error("mysql error: {0}")]
    MySql(#[from] sqlx::Error),
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

impl MySqlStore {
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, MySqlStoreError> {
        let pool = MySqlPoolOptions::new()
            .max_connections(8)
            .connect(url.as_ref())
            .await?;
        Ok(Self {
            pool,
            audit_retention_secs: None,
            audit_last_retention_reap_ms: Arc::new(AtomicI64::new(0)),
        })
    }

    pub fn with_audit_retention_secs(mut self, secs: Option<u64>) -> Self {
        self.audit_retention_secs = secs.filter(|value| *value > 0);
        self
    }

    pub async fn ping(&self) -> Result<(), MySqlStoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn init(&self) -> Result<(), MySqlStoreError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS virtual_keys (
                id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                value_json JSON NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config_state (
                `key` VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                value_json JSON NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_logs (
                id BIGINT AUTO_INCREMENT PRIMARY KEY,
                ts_ms BIGINT NOT NULL,
                kind VARCHAR(255) NOT NULL,
                payload_json JSON NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS budget_ledger (
                key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                spent_tokens BIGINT NOT NULL DEFAULT 0,
                reserved_tokens BIGINT NOT NULL DEFAULT 0,
                updated_at_ms BIGINT NOT NULL,
                CONSTRAINT ck_budget_ledger_spent_nonneg CHECK (spent_tokens >= 0),
                CONSTRAINT ck_budget_ledger_reserved_nonneg CHECK (reserved_tokens >= 0),
                CONSTRAINT ck_budget_ledger_updated_nonneg CHECK (updated_at_ms >= 0)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS budget_reservations (
                request_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
                tokens BIGINT NOT NULL,
                ts_ms BIGINT NOT NULL,
                CONSTRAINT ck_budget_reservations_tokens_nonneg CHECK (tokens >= 0),
                CONSTRAINT ck_budget_reservations_ts_nonneg CHECK (ts_ms >= 0),
                INDEX idx_budget_reservations_key_id (key_id),
                INDEX idx_budget_reservations_ts_ms (ts_ms)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cost_ledger (
                key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                spent_usd_micros BIGINT NOT NULL DEFAULT 0,
                reserved_usd_micros BIGINT NOT NULL DEFAULT 0,
                updated_at_ms BIGINT NOT NULL,
                CONSTRAINT ck_cost_ledger_spent_nonneg CHECK (spent_usd_micros >= 0),
                CONSTRAINT ck_cost_ledger_reserved_nonneg CHECK (reserved_usd_micros >= 0),
                CONSTRAINT ck_cost_ledger_updated_nonneg CHECK (updated_at_ms >= 0)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cost_reservations (
                request_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin PRIMARY KEY NOT NULL,
                key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
                usd_micros BIGINT NOT NULL,
                ts_ms BIGINT NOT NULL,
                CONSTRAINT ck_cost_reservations_usd_nonneg CHECK (usd_micros >= 0),
                CONSTRAINT ck_cost_reservations_ts_nonneg CHECK (ts_ms >= 0),
                INDEX idx_cost_reservations_key_id (key_id),
                INDEX idx_cost_reservations_ts_ms (ts_ms)
            )",
        )
        .execute(&self.pool)
        .await?;

        // Best-effort in-place upgrades for deployments that created looser schemas earlier.
        sqlx::query(
            "ALTER TABLE virtual_keys
             MODIFY COLUMN id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE virtual_keys
             MODIFY COLUMN value_json JSON NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE config_state
             MODIFY COLUMN `key` VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE config_state
             MODIFY COLUMN value_json JSON NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE audit_logs
             MODIFY COLUMN payload_json JSON NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE budget_ledger
             MODIFY COLUMN key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE budget_reservations
             MODIFY COLUMN request_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE budget_reservations
             MODIFY COLUMN key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE cost_ledger
             MODIFY COLUMN key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE cost_reservations
             MODIFY COLUMN request_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE cost_reservations
             MODIFY COLUMN key_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL",
        )
        .execute(&self.pool)
        .await?;

        ensure_mysql_index(
            &self.pool,
            "audit_logs",
            "idx_audit_logs_ts_ms",
            "CREATE INDEX idx_audit_logs_ts_ms ON audit_logs(ts_ms)",
        )
        .await?;
        ensure_mysql_index(
            &self.pool,
            "audit_logs",
            "idx_audit_logs_kind_ts_ms",
            "CREATE INDEX idx_audit_logs_kind_ts_ms ON audit_logs(kind, ts_ms)",
        )
        .await?;
        ensure_mysql_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_key_id",
            "CREATE INDEX idx_budget_reservations_key_id ON budget_reservations(key_id)",
        )
        .await?;
        ensure_mysql_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_ts_ms",
            "CREATE INDEX idx_budget_reservations_ts_ms ON budget_reservations(ts_ms)",
        )
        .await?;
        ensure_mysql_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_key_id",
            "CREATE INDEX idx_cost_reservations_key_id ON cost_reservations(key_id)",
        )
        .await?;
        ensure_mysql_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_ts_ms",
            "CREATE INDEX idx_cost_reservations_ts_ms ON cost_reservations(ts_ms)",
        )
        .await?;

        ensure_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_spent_nonneg",
            "CHECK (spent_tokens >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_reserved_nonneg",
            "CHECK (reserved_tokens >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_updated_nonneg",
            "CHECK (updated_at_ms >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_tokens_nonneg",
            "CHECK (tokens >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_ts_nonneg",
            "CHECK (ts_ms >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_spent_nonneg",
            "CHECK (spent_usd_micros >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_reserved_nonneg",
            "CHECK (reserved_usd_micros >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_updated_nonneg",
            "CHECK (updated_at_ms >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_usd_nonneg",
            "CHECK (usd_micros >= 0)",
        )
        .await?;
        ensure_mysql_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_ts_nonneg",
            "CHECK (ts_ms >= 0)",
        )
        .await?;

        Ok(())
    }

    pub async fn verify_schema(&self) -> Result<(), MySqlStoreError> {
        require_mysql_table(&self.pool, "virtual_keys").await?;
        require_mysql_table(&self.pool, "config_state").await?;
        require_mysql_table(&self.pool, "audit_logs").await?;
        require_mysql_table(&self.pool, "budget_ledger").await?;
        require_mysql_table(&self.pool, "budget_reservations").await?;
        require_mysql_table(&self.pool, "cost_ledger").await?;
        require_mysql_table(&self.pool, "cost_reservations").await?;

        require_mysql_column_data_type(&self.pool, "virtual_keys", "value_json", "json").await?;
        require_mysql_column_data_type(&self.pool, "config_state", "value_json", "json").await?;
        require_mysql_column_data_type(&self.pool, "audit_logs", "payload_json", "json").await?;

        require_mysql_column_collation(&self.pool, "virtual_keys", "id", "utf8mb4_bin").await?;
        require_mysql_column_collation(&self.pool, "config_state", "key", "utf8mb4_bin").await?;
        require_mysql_column_collation(&self.pool, "budget_ledger", "key_id", "utf8mb4_bin")
            .await?;
        require_mysql_column_collation(
            &self.pool,
            "budget_reservations",
            "request_id",
            "utf8mb4_bin",
        )
        .await?;
        require_mysql_column_collation(&self.pool, "budget_reservations", "key_id", "utf8mb4_bin")
            .await?;
        require_mysql_column_collation(&self.pool, "cost_ledger", "key_id", "utf8mb4_bin").await?;
        require_mysql_column_collation(
            &self.pool,
            "cost_reservations",
            "request_id",
            "utf8mb4_bin",
        )
        .await?;
        require_mysql_column_collation(&self.pool, "cost_reservations", "key_id", "utf8mb4_bin")
            .await?;

        require_mysql_index(&self.pool, "audit_logs", "idx_audit_logs_ts_ms").await?;
        require_mysql_index(&self.pool, "audit_logs", "idx_audit_logs_kind_ts_ms").await?;
        require_mysql_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_key_id",
        )
        .await?;
        require_mysql_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_ts_ms",
        )
        .await?;
        require_mysql_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_key_id",
        )
        .await?;
        require_mysql_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_ts_ms",
        )
        .await?;

        require_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_spent_nonneg",
        )
        .await?;
        require_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_reserved_nonneg",
        )
        .await?;
        require_mysql_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_updated_nonneg",
        )
        .await?;
        require_mysql_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_tokens_nonneg",
        )
        .await?;
        require_mysql_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_ts_nonneg",
        )
        .await?;
        require_mysql_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_spent_nonneg")
            .await?;
        require_mysql_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_reserved_nonneg")
            .await?;
        require_mysql_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_updated_nonneg")
            .await?;
        require_mysql_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_usd_nonneg",
        )
        .await?;
        require_mysql_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_ts_nonneg",
        )
        .await?;

        Ok(())
    }

    pub async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyConfig>, MySqlStoreError> {
        let rows = sqlx::query(
            "SELECT CAST(value_json AS CHAR) AS value_json
             FROM virtual_keys
             ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row.try_get("value_json")?;
            keys.push(serde_json::from_str::<VirtualKeyConfig>(&raw)?);
        }
        Ok(keys)
    }

    pub async fn replace_virtual_keys(
        &self,
        keys: &[VirtualKeyConfig],
    ) -> Result<(), MySqlStoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM virtual_keys")
            .execute(&mut *tx)
            .await?;
        for key in keys {
            let value_json = serde_json::to_string(key)?;
            sqlx::query("INSERT INTO virtual_keys (id, value_json) VALUES (?, ?)")
                .bind(&key.id)
                .bind(value_json)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn load_router_config(&self) -> Result<Option<RouterConfig>, MySqlStoreError> {
        let row = sqlx::query(
            "SELECT CAST(value_json AS CHAR) AS value_json
             FROM config_state
             WHERE `key`='router'",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: String = row.try_get("value_json")?;
        Ok(Some(serde_json::from_str::<RouterConfig>(&raw)?))
    }

    pub async fn replace_router_config(
        &self,
        router: &RouterConfig,
    ) -> Result<(), MySqlStoreError> {
        let value_json = serde_json::to_string(router)?;
        sqlx::query(
            "INSERT INTO config_state (`key`, value_json) VALUES ('router', ?)
             ON DUPLICATE KEY UPDATE value_json = VALUES(value_json)",
        )
        .bind(value_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn reserve_budget_tokens(
        &self,
        request_id: &str,
        key_id: &str,
        limit: u64,
        tokens: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let tokens_i64 = u64_to_i64(tokens);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query(
            "SELECT spent_tokens, reserved_tokens FROM budget_ledger WHERE key_id=? FOR UPDATE",
        )
        .bind(key_id)
        .fetch_one(&mut *tx)
        .await?;
        let spent_tokens: i64 = row.try_get("spent_tokens")?;
        let reserved_tokens: i64 = row.try_get("reserved_tokens")?;
        let attempted = i64_to_u64(spent_tokens)
            .saturating_add(i64_to_u64(reserved_tokens))
            .saturating_add(tokens);
        if attempted > limit {
            return Err(MySqlStoreError::BudgetExceeded { limit, attempted });
        }

        sqlx::query(
            "INSERT INTO budget_reservations (request_id, key_id, tokens, ts_ms)
             VALUES (?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(tokens_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = reserved_tokens + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(tokens_i64)
        .bind(ts_ms)
        .bind(key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn reserve_cost_usd_micros(
        &self,
        request_id: &str,
        key_id: &str,
        limit_usd_micros: u64,
        usd_micros: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let usd_i64 = u64_to_i64(usd_micros);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query(
            "SELECT spent_usd_micros, reserved_usd_micros
             FROM cost_ledger
             WHERE key_id=?
             FOR UPDATE",
        )
        .bind(key_id)
        .fetch_one(&mut *tx)
        .await?;
        let spent_usd: i64 = row.try_get("spent_usd_micros")?;
        let reserved_usd: i64 = row.try_get("reserved_usd_micros")?;
        let attempted = i64_to_u64(spent_usd)
            .saturating_add(i64_to_u64(reserved_usd))
            .saturating_add(usd_micros);
        if attempted > limit_usd_micros {
            return Err(MySqlStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros: attempted,
            });
        }

        sqlx::query(
            "INSERT INTO cost_reservations (request_id, key_id, usd_micros, ts_ms)
             VALUES (?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(usd_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = reserved_usd_micros + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(usd_i64)
        .bind(ts_ms)
        .bind(key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_budget_reservation_with_tokens(
        &self,
        request_id: &str,
        spent_tokens: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let spent_tokens_i64 = u64_to_i64(spent_tokens);
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, tokens
             FROM budget_reservations
             WHERE request_id=?
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(reservation) = reservation else {
            return Ok(());
        };
        let key_id: String = reservation.try_get("key_id")?;
        let reserved_tokens_i64: i64 = reservation.try_get("tokens")?;

        sqlx::query("DELETE FROM budget_reservations WHERE request_id=?")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let reserved_i64 = reserved_tokens_i64.max(0);
        let committed_i64 = reserved_i64.min(spent_tokens_i64);
        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = GREATEST(reserved_tokens - ?, 0),
                 spent_tokens = spent_tokens + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(reserved_i64)
        .bind(committed_i64)
        .bind(ts_ms)
        .bind(&key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_budget_reservation(&self, request_id: &str) -> Result<(), MySqlStoreError> {
        self.commit_budget_reservation_with_tokens(request_id, u64::MAX)
            .await
    }

    pub async fn commit_cost_reservation_with_usd_micros(
        &self,
        request_id: &str,
        spent_usd_micros: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let spent_usd_i64 = u64_to_i64(spent_usd_micros);
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, usd_micros
             FROM cost_reservations
             WHERE request_id=?
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(reservation) = reservation else {
            return Ok(());
        };
        let key_id: String = reservation.try_get("key_id")?;
        let reserved_usd_i64: i64 = reservation.try_get("usd_micros")?;

        sqlx::query("DELETE FROM cost_reservations WHERE request_id=?")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let reserved_i64 = reserved_usd_i64.max(0);
        let committed_i64 = reserved_i64.min(spent_usd_i64);
        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = GREATEST(reserved_usd_micros - ?, 0),
                 spent_usd_micros = spent_usd_micros + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(reserved_i64)
        .bind(committed_i64)
        .bind(ts_ms)
        .bind(&key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_cost_reservation(&self, request_id: &str) -> Result<(), MySqlStoreError> {
        self.commit_cost_reservation_with_usd_micros(request_id, u64::MAX)
            .await
    }

    pub async fn rollback_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, tokens
             FROM budget_reservations
             WHERE request_id=?
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(reservation) = reservation else {
            return Ok(());
        };
        let key_id: String = reservation.try_get("key_id")?;
        let reserved_i64: i64 = reservation.try_get("tokens")?;

        sqlx::query("DELETE FROM budget_reservations WHERE request_id=?")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = GREATEST(reserved_tokens - ?, 0),
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(reserved_i64.max(0))
        .bind(ts_ms)
        .bind(&key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn rollback_cost_reservation(&self, request_id: &str) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, usd_micros
             FROM cost_reservations
             WHERE request_id=?
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(reservation) = reservation else {
            return Ok(());
        };
        let key_id: String = reservation.try_get("key_id")?;
        let reserved_i64: i64 = reservation.try_get("usd_micros")?;

        sqlx::query("DELETE FROM cost_reservations WHERE request_id=?")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = GREATEST(reserved_usd_micros - ?, 0),
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(reserved_i64.max(0))
        .bind(ts_ms)
        .bind(&key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn record_spent_tokens(
        &self,
        key_id: &str,
        tokens: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let tokens_i64 = u64_to_i64(tokens);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET spent_tokens = spent_tokens + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(tokens_i64)
        .bind(ts_ms)
        .bind(key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn record_spent_cost_usd_micros(
        &self,
        key_id: &str,
        usd_micros: u64,
    ) -> Result<(), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let usd_i64 = u64_to_i64(usd_micros);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES (?, 0, 0, ?)
             ON DUPLICATE KEY UPDATE key_id = key_id",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET spent_usd_micros = spent_usd_micros + ?,
                 updated_at_ms = ?
             WHERE key_id = ?",
        )
        .bind(usd_i64)
        .bind(ts_ms)
        .bind(key_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn list_budget_ledgers(&self) -> Result<Vec<BudgetLedgerRecord>, MySqlStoreError> {
        let rows = sqlx::query(
            "SELECT key_id, spent_tokens, reserved_tokens, updated_at_ms
             FROM budget_ledger
             ORDER BY key_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let key_id: String = row.try_get("key_id")?;
            let spent_tokens: i64 = row.try_get("spent_tokens")?;
            let reserved_tokens: i64 = row.try_get("reserved_tokens")?;
            let updated_at_ms: i64 = row.try_get("updated_at_ms")?;
            out.push(BudgetLedgerRecord {
                key_id,
                spent_tokens: i64_to_u64(spent_tokens),
                reserved_tokens: i64_to_u64(reserved_tokens),
                updated_at_ms: i64_to_u64(updated_at_ms),
            });
        }

        Ok(out)
    }

    pub async fn list_cost_ledgers(&self) -> Result<Vec<CostLedgerRecord>, MySqlStoreError> {
        let rows = sqlx::query(
            "SELECT key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms
             FROM cost_ledger
             ORDER BY key_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let key_id: String = row.try_get("key_id")?;
            let spent_usd_micros: i64 = row.try_get("spent_usd_micros")?;
            let reserved_usd_micros: i64 = row.try_get("reserved_usd_micros")?;
            let updated_at_ms: i64 = row.try_get("updated_at_ms")?;
            out.push(CostLedgerRecord {
                key_id,
                spent_usd_micros: i64_to_u64(spent_usd_micros),
                reserved_usd_micros: i64_to_u64(reserved_usd_micros),
                updated_at_ms: i64_to_u64(updated_at_ms),
            });
        }

        Ok(out)
    }

    pub async fn reap_stale_budget_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let cutoff_i64 = u64_to_i64(cutoff_ts_ms);
        let limit_i64 = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);

        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT request_id, key_id, tokens
             FROM budget_reservations
             WHERE ts_ms < ?
             ORDER BY ts_ms ASC
             LIMIT ?
             FOR UPDATE",
        )
        .bind(cutoff_i64)
        .bind(limit_i64)
        .fetch_all(&mut *tx)
        .await?;

        let scanned = rows.len() as u64;
        let mut reaped = 0u64;
        let mut released_tokens = 0u64;

        for row in rows {
            let request_id: String = row.try_get("request_id")?;
            let key_id: String = row.try_get("key_id")?;
            let tokens_i64: i64 = row.try_get("tokens")?;
            let tokens_u64 = i64_to_u64(tokens_i64);
            released_tokens = released_tokens.saturating_add(tokens_u64);
            reaped = reaped.saturating_add(1);

            if dry_run {
                continue;
            }

            sqlx::query("DELETE FROM budget_reservations WHERE request_id=?")
                .bind(request_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES (?, 0, 0, ?)
                 ON DUPLICATE KEY UPDATE key_id = key_id",
            )
            .bind(&key_id)
            .bind(ts_ms)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE budget_ledger
                 SET reserved_tokens = GREATEST(reserved_tokens - ?, 0),
                     updated_at_ms = ?
                 WHERE key_id = ?",
            )
            .bind(tokens_i64.max(0))
            .bind(ts_ms)
            .bind(&key_id)
            .execute(&mut *tx)
            .await?;
        }

        if !dry_run {
            tx.commit().await?;
        }

        Ok((scanned, reaped, released_tokens))
    }

    pub async fn reap_stale_cost_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), MySqlStoreError> {
        let ts_ms = now_millis_i64();
        let cutoff_i64 = u64_to_i64(cutoff_ts_ms);
        let limit_i64 = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);

        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT request_id, key_id, usd_micros
             FROM cost_reservations
             WHERE ts_ms < ?
             ORDER BY ts_ms ASC
             LIMIT ?
             FOR UPDATE",
        )
        .bind(cutoff_i64)
        .bind(limit_i64)
        .fetch_all(&mut *tx)
        .await?;

        let scanned = rows.len() as u64;
        let mut reaped = 0u64;
        let mut released_usd_micros = 0u64;

        for row in rows {
            let request_id: String = row.try_get("request_id")?;
            let key_id: String = row.try_get("key_id")?;
            let usd_i64: i64 = row.try_get("usd_micros")?;
            let usd_u64 = i64_to_u64(usd_i64);
            released_usd_micros = released_usd_micros.saturating_add(usd_u64);
            reaped = reaped.saturating_add(1);

            if dry_run {
                continue;
            }

            sqlx::query("DELETE FROM cost_reservations WHERE request_id=?")
                .bind(request_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES (?, 0, 0, ?)
                 ON DUPLICATE KEY UPDATE key_id = key_id",
            )
            .bind(&key_id)
            .bind(ts_ms)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE cost_ledger
                 SET reserved_usd_micros = GREATEST(reserved_usd_micros - ?, 0),
                     updated_at_ms = ?
                 WHERE key_id = ?",
            )
            .bind(usd_i64.max(0))
            .bind(ts_ms)
            .bind(&key_id)
            .execute(&mut *tx)
            .await?;
        }

        if !dry_run {
            tx.commit().await?;
        }

        Ok((scanned, reaped, released_usd_micros))
    }

    pub async fn append_audit_log(
        &self,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<(), MySqlStoreError> {
        let kind = kind.into();
        let payload_json = serde_json::to_string(&payload)?;
        let ts_ms = now_millis_i64();

        sqlx::query("INSERT INTO audit_logs (ts_ms, kind, payload_json) VALUES (?, ?, ?)")
            .bind(ts_ms)
            .bind(kind)
            .bind(payload_json)
            .execute(&self.pool)
            .await?;

        if let Some(cutoff_ms) = audit_cutoff_ms(self.audit_retention_secs, ts_ms)
            && should_run_retention_reap(&self.audit_last_retention_reap_ms, ts_ms)
        {
            let _ = sqlx::query("DELETE FROM audit_logs WHERE ts_ms < ?")
                .bind(cutoff_ms)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    pub async fn reap_audit_logs_before(&self, cutoff_ts_ms: u64) -> Result<u64, MySqlStoreError> {
        let cutoff_ts_ms = u64_to_i64(cutoff_ts_ms);
        let deleted = sqlx::query("DELETE FROM audit_logs WHERE ts_ms < ?")
            .bind(cutoff_ts_ms)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted)
    }

    pub async fn list_audit_logs(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, MySqlStoreError> {
        let limit_i64 = i64::try_from(limit.clamp(1, 1000)).unwrap_or(i64::MAX);

        let rows = if let Some(since_ts_ms) = since_ts_ms {
            let since_i64 = u64_to_i64(since_ts_ms);
            sqlx::query(
                "SELECT id, ts_ms, kind, CAST(payload_json AS CHAR) AS payload_json
                 FROM audit_logs
                 WHERE ts_ms >= ?
                 ORDER BY id DESC
                 LIMIT ?",
            )
            .bind(since_i64)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, ts_ms, kind, CAST(payload_json AS CHAR) AS payload_json
                 FROM audit_logs
                 ORDER BY id DESC
                 LIMIT ?",
            )
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let ts_ms: i64 = row.try_get("ts_ms")?;
            let kind: String = row.try_get("kind")?;
            let payload_json: String = row.try_get("payload_json")?;
            out.push(AuditLogRecord {
                id,
                ts_ms: i64_to_u64(ts_ms),
                kind,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        Ok(out)
    }

    pub async fn list_audit_logs_window(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
        before_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, MySqlStoreError> {
        let limit_i64 = i64::try_from(limit.clamp(1, 10_000)).unwrap_or(i64::MAX);
        let since_i64 = since_ts_ms.map(u64_to_i64).unwrap_or(0);
        let before_i64 = before_ts_ms.map(u64_to_i64).unwrap_or(i64::MAX);

        let rows = sqlx::query(
            "SELECT id, ts_ms, kind, CAST(payload_json AS CHAR) AS payload_json
             FROM audit_logs
             WHERE ts_ms >= ? AND ts_ms < ?
             ORDER BY id DESC
             LIMIT ?",
        )
        .bind(since_i64)
        .bind(before_i64)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let ts_ms: i64 = row.try_get("ts_ms")?;
            let kind: String = row.try_get("kind")?;
            let payload_json: String = row.try_get("payload_json")?;
            out.push(AuditLogRecord {
                id,
                ts_ms: i64_to_u64(ts_ms),
                kind,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        Ok(out)
    }
}

async fn ensure_mysql_index(
    pool: &sqlx::MySqlPool,
    table: &str,
    index: &str,
    create_ddl: &str,
) -> Result<(), MySqlStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND INDEX_NAME = ?
         LIMIT 1",
    )
    .bind(table)
    .bind(index)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        return Ok(());
    }

    sqlx::query(create_ddl).execute(pool).await?;
    Ok(())
}

async fn ensure_mysql_check_constraint(
    pool: &sqlx::MySqlPool,
    table: &str,
    constraint: &str,
    check_expr: &str,
) -> Result<(), MySqlStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS
         WHERE CONSTRAINT_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND CONSTRAINT_NAME = ?
           AND CONSTRAINT_TYPE = 'CHECK'
         LIMIT 1",
    )
    .bind(table)
    .bind(constraint)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        return Ok(());
    }

    let ddl = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} {check_expr}",
        quote_mysql_ident(table),
        quote_mysql_ident(constraint),
    );
    sqlx::query(&ddl).execute(pool).await?;
    Ok(())
}

async fn require_mysql_table(pool: &sqlx::MySqlPool, table: &str) -> Result<(), MySqlStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM INFORMATION_SCHEMA.TABLES
         WHERE TABLE_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
         LIMIT 1",
    )
    .bind(table)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        Ok(())
    } else {
        Err(MySqlStoreError::Schema(format!(
            "missing table `{table}` in active database"
        )))
    }
}

async fn require_mysql_column_data_type(
    pool: &sqlx::MySqlPool,
    table: &str,
    column: &str,
    expected_data_type: &str,
) -> Result<(), MySqlStoreError> {
    let row = sqlx::query(
        "SELECT DATA_TYPE
         FROM INFORMATION_SCHEMA.COLUMNS
         WHERE TABLE_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND COLUMN_NAME = ?",
    )
    .bind(table)
    .bind(column)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(MySqlStoreError::Schema(format!(
            "missing column `{table}.{column}`"
        )));
    };

    let data_type: String = row.try_get("DATA_TYPE")?;
    if data_type.eq_ignore_ascii_case(expected_data_type) {
        Ok(())
    } else {
        Err(MySqlStoreError::Schema(format!(
            "column `{table}.{column}` has data type `{data_type}`, expected `{expected_data_type}`"
        )))
    }
}

async fn require_mysql_column_collation(
    pool: &sqlx::MySqlPool,
    table: &str,
    column: &str,
    expected_collation: &str,
) -> Result<(), MySqlStoreError> {
    let row = sqlx::query(
        "SELECT COLLATION_NAME
         FROM INFORMATION_SCHEMA.COLUMNS
         WHERE TABLE_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND COLUMN_NAME = ?",
    )
    .bind(table)
    .bind(column)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(MySqlStoreError::Schema(format!(
            "missing column `{table}.{column}`"
        )));
    };

    let collation: Option<String> = row.try_get("COLLATION_NAME")?;
    let Some(collation) = collation else {
        return Err(MySqlStoreError::Schema(format!(
            "column `{table}.{column}` has no collation, expected `{expected_collation}`"
        )));
    };
    if collation.eq_ignore_ascii_case(expected_collation) {
        Ok(())
    } else {
        Err(MySqlStoreError::Schema(format!(
            "column `{table}.{column}` has collation `{collation}`, expected `{expected_collation}`"
        )))
    }
}

async fn require_mysql_index(
    pool: &sqlx::MySqlPool,
    table: &str,
    index: &str,
) -> Result<(), MySqlStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND INDEX_NAME = ?
         LIMIT 1",
    )
    .bind(table)
    .bind(index)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        Ok(())
    } else {
        Err(MySqlStoreError::Schema(format!(
            "missing index `{index}` on table `{table}`"
        )))
    }
}

async fn require_mysql_check_constraint(
    pool: &sqlx::MySqlPool,
    table: &str,
    constraint: &str,
) -> Result<(), MySqlStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS
         WHERE CONSTRAINT_SCHEMA = DATABASE()
           AND TABLE_NAME = ?
           AND CONSTRAINT_NAME = ?
           AND CONSTRAINT_TYPE = 'CHECK'
         LIMIT 1",
    )
    .bind(table)
    .bind(constraint)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        Ok(())
    } else {
        Err(MySqlStoreError::Schema(format!(
            "missing check constraint `{constraint}` on table `{table}`"
        )))
    }
}

fn quote_mysql_ident(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn audit_cutoff_ms(retention_secs: Option<u64>, now_ms: i64) -> Option<i64> {
    let retention_secs = retention_secs?;
    let retention_ms = retention_secs.saturating_mul(1000);
    let retention_ms = i64::try_from(retention_ms).unwrap_or(i64::MAX);
    Some(now_ms.saturating_sub(retention_ms))
}

fn should_run_retention_reap(last_reap_ms: &AtomicI64, now_ms: i64) -> bool {
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

fn now_millis_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn u64_to_i64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

fn i64_to_u64(value: i64) -> u64 {
    if value <= 0 { 0 } else { value as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn env_nonempty(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn mysql_url() -> Option<String> {
        env_nonempty("DITTO_MYSQL_URL").or_else(|| env_nonempty("MYSQL_URL"))
    }

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_suffix() -> String {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", now_millis_i64())
    }

    #[tokio::test]
    async fn mysql_store_round_trips_config_audit_and_ledgers() {
        let url = mysql_url()
            .unwrap_or_else(|| panic!("missing mysql test url; set DITTO_MYSQL_URL or MYSQL_URL"));

        let store = MySqlStore::connect(url).await.expect("connect");
        store.ping().await.expect("ping");
        store.init().await.expect("init");
        store.verify_schema().await.expect("verify schema");

        let suffix = test_suffix();
        let key_id = format!("my-key-{suffix}");
        let token = format!("my-token-{suffix}");
        let request_id_budget = format!("my-budget-{suffix}");
        let request_id_cost = format!("my-cost-{suffix}");
        let request_id_reap_budget = format!("my-reap-budget-{suffix}");
        let request_id_reap_cost = format!("my-reap-cost-{suffix}");

        let key = VirtualKeyConfig::new(&key_id, &token);
        store
            .replace_virtual_keys(std::slice::from_ref(&key))
            .await
            .expect("replace virtual keys");
        let loaded_keys = store.load_virtual_keys().await.expect("load virtual keys");
        assert_eq!(loaded_keys.len(), 1);
        assert_eq!(loaded_keys[0].id, key_id);

        let router = RouterConfig {
            default_backends: Vec::new(),
            rules: Vec::new(),
        };
        store
            .replace_router_config(&router)
            .await
            .expect("replace router");
        let loaded_router = store.load_router_config().await.expect("load router");
        assert!(loaded_router.is_some());

        store
            .append_audit_log("mysql.test", serde_json::json!({ "suffix": suffix }))
            .await
            .expect("append audit log");
        let logs = store
            .list_audit_logs(50, None)
            .await
            .expect("list audit logs");
        assert!(logs.iter().any(|record| record.kind == "mysql.test"));

        store
            .reserve_budget_tokens(&request_id_budget, &key_id, 20, 7)
            .await
            .expect("reserve budget");
        store
            .commit_budget_reservation_with_tokens(&request_id_budget, 3)
            .await
            .expect("commit budget");

        let budget_ledgers = store.list_budget_ledgers().await.expect("budget ledgers");
        let budget_ledger = budget_ledgers
            .iter()
            .find(|ledger| ledger.key_id == key_id)
            .expect("budget ledger for key");
        assert_eq!(budget_ledger.spent_tokens, 3);
        assert_eq!(budget_ledger.reserved_tokens, 0);

        store
            .reserve_cost_usd_micros(&request_id_cost, &key_id, 20, 9)
            .await
            .expect("reserve cost");
        store
            .commit_cost_reservation_with_usd_micros(&request_id_cost, 4)
            .await
            .expect("commit cost");

        let cost_ledgers = store.list_cost_ledgers().await.expect("cost ledgers");
        let cost_ledger = cost_ledgers
            .iter()
            .find(|ledger| ledger.key_id == key_id)
            .expect("cost ledger for key");
        assert_eq!(cost_ledger.spent_usd_micros, 4);
        assert_eq!(cost_ledger.reserved_usd_micros, 0);

        store
            .reserve_budget_tokens(&request_id_reap_budget, &key_id, 100, 5)
            .await
            .expect("reserve reap budget");
        store
            .reserve_cost_usd_micros(&request_id_reap_cost, &key_id, 100, 6)
            .await
            .expect("reserve reap cost");

        let _ = store
            .reap_stale_budget_reservations(u64::MAX, 1000, false)
            .await
            .expect("reap budget");
        let _ = store
            .reap_stale_cost_reservations(u64::MAX, 1000, false)
            .await
            .expect("reap cost");

        let budget_ledgers = store
            .list_budget_ledgers()
            .await
            .expect("budget ledgers after reap");
        let budget_ledger = budget_ledgers
            .iter()
            .find(|ledger| ledger.key_id == key_id)
            .expect("budget ledger after reap");
        assert_eq!(budget_ledger.reserved_tokens, 0);

        let cost_ledgers = store
            .list_cost_ledgers()
            .await
            .expect("cost ledgers after reap");
        let cost_ledger = cost_ledgers
            .iter()
            .find(|ledger| ledger.key_id == key_id)
            .expect("cost ledger after reap");
        assert_eq!(cost_ledger.reserved_usd_micros, 0);
    }
}
