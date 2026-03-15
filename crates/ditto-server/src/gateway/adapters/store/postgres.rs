use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use thiserror::Error;

use super::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, ProxyRequestFingerprint,
    ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestReplayOutcome, RouterConfig, VirtualKeyConfig,
};

#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: sqlx::PgPool,
    audit_retention_secs: Option<u64>,
    audit_last_retention_reap_ms: Arc<AtomicI64>,
}

const AUDIT_RETENTION_REAP_INTERVAL_MS: i64 = 30_000;

#[derive(Debug, Error)]
pub enum PostgresStoreError {
    #[error("postgres error: {0}")]
    Postgres(#[from] sqlx::Error),
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

impl PostgresStore {
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, PostgresStoreError> {
        let pool = PgPoolOptions::new()
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

    pub async fn ping(&self) -> Result<(), PostgresStoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn init(&self) -> Result<(), PostgresStoreError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS virtual_keys (
                id TEXT PRIMARY KEY NOT NULL,
                value_json JSONB NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config_state (
                key TEXT PRIMARY KEY NOT NULL,
                value_json JSONB NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_logs (
                id BIGSERIAL PRIMARY KEY,
                ts_ms BIGINT NOT NULL,
                kind TEXT NOT NULL,
                payload_json JSONB NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS budget_ledger (
                key_id TEXT PRIMARY KEY NOT NULL,
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
                request_id TEXT PRIMARY KEY NOT NULL,
                key_id TEXT NOT NULL,
                tokens BIGINT NOT NULL,
                ts_ms BIGINT NOT NULL,
                CONSTRAINT ck_budget_reservations_tokens_nonneg CHECK (tokens >= 0),
                CONSTRAINT ck_budget_reservations_ts_nonneg CHECK (ts_ms >= 0)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_budget_reservations_key_id
             ON budget_reservations(key_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_budget_reservations_ts_ms
             ON budget_reservations(ts_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cost_ledger (
                key_id TEXT PRIMARY KEY NOT NULL,
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
                request_id TEXT PRIMARY KEY NOT NULL,
                key_id TEXT NOT NULL,
                usd_micros BIGINT NOT NULL,
                ts_ms BIGINT NOT NULL,
                CONSTRAINT ck_cost_reservations_usd_nonneg CHECK (usd_micros >= 0),
                CONSTRAINT ck_cost_reservations_ts_nonneg CHECK (ts_ms >= 0)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cost_reservations_key_id
             ON cost_reservations(key_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cost_reservations_ts_ms
             ON cost_reservations(ts_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_ts_ms
             ON audit_logs(ts_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_kind_ts_ms
             ON audit_logs(kind, ts_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS proxy_request_idempotency (
                request_id TEXT PRIMARY KEY NOT NULL,
                state TEXT NOT NULL,
                owner_token TEXT,
                lease_until_ms BIGINT,
                expires_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                record_json JSONB NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_proxy_request_idempotency_expires_at_ms
             ON proxy_request_idempotency(expires_at_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_proxy_request_idempotency_state_lease_until_ms
             ON proxy_request_idempotency(state, lease_until_ms)",
        )
        .execute(&self.pool)
        .await?;

        // Best-effort in-place upgrades for deployments that created TEXT columns earlier.
        sqlx::query(
            "ALTER TABLE virtual_keys
             ALTER COLUMN value_json TYPE JSONB
             USING value_json::jsonb",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE config_state
             ALTER COLUMN value_json TYPE JSONB
             USING value_json::jsonb",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE audit_logs
             ALTER COLUMN payload_json TYPE JSONB
             USING payload_json::jsonb",
        )
        .execute(&self.pool)
        .await?;

        ensure_pg_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_spent_nonneg",
            "CHECK (spent_tokens >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_reserved_nonneg",
            "CHECK (reserved_tokens >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_updated_nonneg",
            "CHECK (updated_at_ms >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_tokens_nonneg",
            "CHECK (tokens >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_ts_nonneg",
            "CHECK (ts_ms >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_spent_nonneg",
            "CHECK (spent_usd_micros >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_reserved_nonneg",
            "CHECK (reserved_usd_micros >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "cost_ledger",
            "ck_cost_ledger_updated_nonneg",
            "CHECK (updated_at_ms >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_usd_nonneg",
            "CHECK (usd_micros >= 0)",
        )
        .await?;
        ensure_pg_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_ts_nonneg",
            "CHECK (ts_ms >= 0)",
        )
        .await?;

        Ok(())
    }

    pub async fn verify_schema(&self) -> Result<(), PostgresStoreError> {
        require_pg_table(&self.pool, "virtual_keys").await?;
        require_pg_table(&self.pool, "config_state").await?;
        require_pg_table(&self.pool, "audit_logs").await?;
        require_pg_table(&self.pool, "budget_ledger").await?;
        require_pg_table(&self.pool, "budget_reservations").await?;
        require_pg_table(&self.pool, "cost_ledger").await?;
        require_pg_table(&self.pool, "cost_reservations").await?;
        require_pg_table(&self.pool, "proxy_request_idempotency").await?;

        require_pg_column_udt(&self.pool, "virtual_keys", "value_json", "jsonb").await?;
        require_pg_column_udt(&self.pool, "config_state", "value_json", "jsonb").await?;
        require_pg_column_udt(&self.pool, "audit_logs", "payload_json", "jsonb").await?;

        require_pg_index(&self.pool, "audit_logs", "idx_audit_logs_ts_ms").await?;
        require_pg_index(&self.pool, "audit_logs", "idx_audit_logs_kind_ts_ms").await?;
        require_pg_index(
            &self.pool,
            "proxy_request_idempotency",
            "idx_proxy_request_idempotency_expires_at_ms",
        )
        .await?;
        require_pg_index(
            &self.pool,
            "proxy_request_idempotency",
            "idx_proxy_request_idempotency_state_lease_until_ms",
        )
        .await?;
        require_pg_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_key_id",
        )
        .await?;
        require_pg_index(
            &self.pool,
            "budget_reservations",
            "idx_budget_reservations_ts_ms",
        )
        .await?;
        require_pg_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_key_id",
        )
        .await?;
        require_pg_index(
            &self.pool,
            "cost_reservations",
            "idx_cost_reservations_ts_ms",
        )
        .await?;

        require_pg_check_constraint(&self.pool, "budget_ledger", "ck_budget_ledger_spent_nonneg")
            .await?;
        require_pg_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_reserved_nonneg",
        )
        .await?;
        require_pg_check_constraint(
            &self.pool,
            "budget_ledger",
            "ck_budget_ledger_updated_nonneg",
        )
        .await?;
        require_pg_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_tokens_nonneg",
        )
        .await?;
        require_pg_check_constraint(
            &self.pool,
            "budget_reservations",
            "ck_budget_reservations_ts_nonneg",
        )
        .await?;
        require_pg_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_spent_nonneg")
            .await?;
        require_pg_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_reserved_nonneg")
            .await?;
        require_pg_check_constraint(&self.pool, "cost_ledger", "ck_cost_ledger_updated_nonneg")
            .await?;
        require_pg_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_usd_nonneg",
        )
        .await?;
        require_pg_check_constraint(
            &self.pool,
            "cost_reservations",
            "ck_cost_reservations_ts_nonneg",
        )
        .await?;

        Ok(())
    }

    pub async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyConfig>, PostgresStoreError> {
        let rows = sqlx::query("SELECT value_json FROM virtual_keys ORDER BY id")
            .fetch_all(&self.pool)
            .await?;

        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            let Json(raw): Json<serde_json::Value> = row.try_get("value_json")?;
            keys.push(serde_json::from_value::<VirtualKeyConfig>(raw)?);
        }
        Ok(keys)
    }

    pub async fn replace_virtual_keys(
        &self,
        keys: &[VirtualKeyConfig],
    ) -> Result<(), PostgresStoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM virtual_keys")
            .execute(&mut *tx)
            .await?;
        for key in keys {
            let key = key.sanitized_for_persistence();
            let value_json = serde_json::to_value(&key)?;
            sqlx::query("INSERT INTO virtual_keys (id, value_json) VALUES ($1, $2)")
                .bind(&key.id)
                .bind(Json(value_json))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn load_router_config(&self) -> Result<Option<RouterConfig>, PostgresStoreError> {
        let row = sqlx::query("SELECT value_json FROM config_state WHERE key='router'")
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let Json(raw): Json<serde_json::Value> = row.try_get("value_json")?;
        Ok(Some(serde_json::from_value::<RouterConfig>(raw)?))
    }

    pub async fn replace_router_config(
        &self,
        router: &RouterConfig,
    ) -> Result<(), PostgresStoreError> {
        let value_json = serde_json::to_value(router)?;
        sqlx::query(
            "INSERT INTO config_state (key, value_json) VALUES ('router', $1)
             ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(Json(value_json))
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
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let tokens_i64 = u64_to_i64(tokens);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query(
            "SELECT spent_tokens, reserved_tokens FROM budget_ledger WHERE key_id=$1 FOR UPDATE",
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
            return Err(PostgresStoreError::BudgetExceeded { limit, attempted });
        }

        sqlx::query(
            "INSERT INTO budget_reservations (request_id, key_id, tokens, ts_ms)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(tokens_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = reserved_tokens + $2,
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(key_id)
        .bind(tokens_i64)
        .bind(ts_ms)
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
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let usd_i64 = u64_to_i64(usd_micros);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query(
            "SELECT spent_usd_micros, reserved_usd_micros
             FROM cost_ledger
             WHERE key_id=$1
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
            return Err(PostgresStoreError::CostBudgetExceeded {
                limit_usd_micros,
                attempted_usd_micros: attempted,
            });
        }

        sqlx::query(
            "INSERT INTO cost_reservations (request_id, key_id, usd_micros, ts_ms)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(usd_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = reserved_usd_micros + $2,
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(key_id)
        .bind(usd_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_budget_reservation_with_tokens(
        &self,
        request_id: &str,
        spent_tokens: u64,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let spent_tokens_i64 = u64_to_i64(spent_tokens);
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, tokens
             FROM budget_reservations
             WHERE request_id=$1
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

        sqlx::query("DELETE FROM budget_reservations WHERE request_id=$1")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let reserved_i64 = reserved_tokens_i64.max(0);
        let committed_i64 = reserved_i64.min(spent_tokens_i64);
        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = GREATEST(reserved_tokens - $2, 0),
                 spent_tokens = spent_tokens + $3,
                 updated_at_ms = $4
             WHERE key_id = $1",
        )
        .bind(&key_id)
        .bind(reserved_i64)
        .bind(committed_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), PostgresStoreError> {
        self.commit_budget_reservation_with_tokens(request_id, u64::MAX)
            .await
    }

    pub async fn commit_cost_reservation_with_usd_micros(
        &self,
        request_id: &str,
        spent_usd_micros: u64,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let spent_usd_i64 = u64_to_i64(spent_usd_micros);
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, usd_micros
             FROM cost_reservations
             WHERE request_id=$1
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

        sqlx::query("DELETE FROM cost_reservations WHERE request_id=$1")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        let reserved_i64 = reserved_usd_i64.max(0);
        let committed_i64 = reserved_i64.min(spent_usd_i64);
        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = GREATEST(reserved_usd_micros - $2, 0),
                 spent_usd_micros = spent_usd_micros + $3,
                 updated_at_ms = $4
             WHERE key_id = $1",
        )
        .bind(&key_id)
        .bind(reserved_i64)
        .bind(committed_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn commit_cost_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), PostgresStoreError> {
        self.commit_cost_reservation_with_usd_micros(request_id, u64::MAX)
            .await
    }

    pub async fn rollback_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, tokens
             FROM budget_reservations
             WHERE request_id=$1
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

        sqlx::query("DELETE FROM budget_reservations WHERE request_id=$1")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET reserved_tokens = GREATEST(reserved_tokens - $2, 0),
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(&key_id)
        .bind(reserved_i64.max(0))
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn rollback_cost_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let mut tx = self.pool.begin().await?;

        let reservation = sqlx::query(
            "SELECT key_id, usd_micros
             FROM cost_reservations
             WHERE request_id=$1
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

        sqlx::query("DELETE FROM cost_reservations WHERE request_id=$1")
            .bind(request_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(&key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET reserved_usd_micros = GREATEST(reserved_usd_micros - $2, 0),
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(&key_id)
        .bind(reserved_i64.max(0))
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn record_spent_tokens(
        &self,
        key_id: &str,
        tokens: u64,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let tokens_i64 = u64_to_i64(tokens);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE budget_ledger
             SET spent_tokens = spent_tokens + $2,
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(key_id)
        .bind(tokens_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn record_spent_cost_usd_micros(
        &self,
        key_id: &str,
        usd_micros: u64,
    ) -> Result<(), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let usd_i64 = u64_to_i64(usd_micros);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
             VALUES ($1, 0, 0, $2)
             ON CONFLICT (key_id) DO NOTHING",
        )
        .bind(key_id)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE cost_ledger
             SET spent_usd_micros = spent_usd_micros + $2,
                 updated_at_ms = $3
             WHERE key_id = $1",
        )
        .bind(key_id)
        .bind(usd_i64)
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn list_budget_ledgers(&self) -> Result<Vec<BudgetLedgerRecord>, PostgresStoreError> {
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

    pub async fn list_cost_ledgers(&self) -> Result<Vec<CostLedgerRecord>, PostgresStoreError> {
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
    ) -> Result<(u64, u64, u64), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let cutoff_i64 = u64_to_i64(cutoff_ts_ms);
        let limit_i64 = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);

        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT request_id, key_id, tokens
             FROM budget_reservations
             WHERE ts_ms < $1
             ORDER BY ts_ms ASC
             LIMIT $2
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

            sqlx::query("DELETE FROM budget_reservations WHERE request_id=$1")
                .bind(request_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO budget_ledger (key_id, spent_tokens, reserved_tokens, updated_at_ms)
                 VALUES ($1, 0, 0, $2)
                 ON CONFLICT (key_id) DO NOTHING",
            )
            .bind(&key_id)
            .bind(ts_ms)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE budget_ledger
                 SET reserved_tokens = GREATEST(reserved_tokens - $2, 0),
                     updated_at_ms = $3
                 WHERE key_id = $1",
            )
            .bind(&key_id)
            .bind(tokens_i64.max(0))
            .bind(ts_ms)
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
    ) -> Result<(u64, u64, u64), PostgresStoreError> {
        let ts_ms = now_millis_i64();
        let cutoff_i64 = u64_to_i64(cutoff_ts_ms);
        let limit_i64 = i64::try_from(max_reaped.clamp(1, 100_000)).unwrap_or(100_000);

        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT request_id, key_id, usd_micros
             FROM cost_reservations
             WHERE ts_ms < $1
             ORDER BY ts_ms ASC
             LIMIT $2
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

            sqlx::query("DELETE FROM cost_reservations WHERE request_id=$1")
                .bind(request_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO cost_ledger (key_id, spent_usd_micros, reserved_usd_micros, updated_at_ms)
                 VALUES ($1, 0, 0, $2)
                 ON CONFLICT (key_id) DO NOTHING",
            )
            .bind(&key_id)
            .bind(ts_ms)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE cost_ledger
                 SET reserved_usd_micros = GREATEST(reserved_usd_micros - $2, 0),
                     updated_at_ms = $3
                 WHERE key_id = $1",
            )
            .bind(&key_id)
            .bind(usd_i64.max(0))
            .bind(ts_ms)
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
    ) -> Result<(), PostgresStoreError> {
        let kind = kind.into();
        let ts_ms = now_millis_i64();

        sqlx::query("INSERT INTO audit_logs (ts_ms, kind, payload_json) VALUES ($1, $2, $3)")
            .bind(ts_ms)
            .bind(kind)
            .bind(Json(payload))
            .execute(&self.pool)
            .await?;

        if let Some(cutoff_ms) = audit_cutoff_ms(self.audit_retention_secs, ts_ms)
            && should_run_retention_reap(&self.audit_last_retention_reap_ms, ts_ms)
        {
            let _ = sqlx::query("DELETE FROM audit_logs WHERE ts_ms < $1")
                .bind(cutoff_ms)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    pub async fn reap_audit_logs_before(
        &self,
        cutoff_ts_ms: u64,
    ) -> Result<u64, PostgresStoreError> {
        let cutoff_ts_ms = u64_to_i64(cutoff_ts_ms);
        let deleted = sqlx::query("DELETE FROM audit_logs WHERE ts_ms < $1")
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
    ) -> Result<Vec<AuditLogRecord>, PostgresStoreError> {
        let limit_i64 = i64::try_from(limit.clamp(1, 1000)).unwrap_or(i64::MAX);

        let rows = if let Some(since_ts_ms) = since_ts_ms {
            let since_i64 = u64_to_i64(since_ts_ms);
            sqlx::query(
                "SELECT id, ts_ms, kind, payload_json
                 FROM audit_logs
                 WHERE ts_ms >= $1
                 ORDER BY id DESC
                 LIMIT $2",
            )
            .bind(since_i64)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, ts_ms, kind, payload_json
                 FROM audit_logs
                 ORDER BY id DESC
                 LIMIT $1",
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
            let Json(payload): Json<serde_json::Value> = row.try_get("payload_json")?;
            out.push(AuditLogRecord {
                id,
                ts_ms: i64_to_u64(ts_ms),
                kind,
                payload,
            });
        }
        Ok(out)
    }

    pub async fn list_audit_logs_window(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
        before_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, PostgresStoreError> {
        let limit_i64 = i64::try_from(limit.clamp(1, 10_000)).unwrap_or(i64::MAX);
        let since_i64 = since_ts_ms.map(u64_to_i64).unwrap_or(0);
        let before_i64 = before_ts_ms.map(u64_to_i64).unwrap_or(i64::MAX);

        let rows = sqlx::query(
            "SELECT id, ts_ms, kind, payload_json
             FROM audit_logs
             WHERE ts_ms >= $1 AND ts_ms < $2
             ORDER BY id DESC
             LIMIT $3",
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
            let Json(payload): Json<serde_json::Value> = row.try_get("payload_json")?;
            out.push(AuditLogRecord {
                id,
                ts_ms: i64_to_u64(ts_ms),
                kind,
                payload,
            });
        }
        Ok(out)
    }

    pub async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, PostgresStoreError> {
        let now_ms_i64 = u64_to_i64(now_ms);
        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        let lease_until_ms_i64 = u64_to_i64(lease_until_ms);
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM proxy_request_idempotency WHERE expires_at_ms < $1")
            .bind(now_ms_i64)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query(
            "SELECT record_json
             FROM proxy_request_idempotency
             WHERE request_id = $1
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = row {
            let Json(record): Json<ProxyRequestIdempotencyRecord> = row.try_get("record_json")?;
            if record.fingerprint_key != fingerprint_key {
                tx.commit().await?;
                return Ok(ProxyRequestIdempotencyBeginOutcome::Conflict { record });
            }
            if record.expires_at_ms >= now_ms {
                match record.state {
                    ProxyRequestIdempotencyState::Completed => {
                        tx.commit().await?;
                        return Ok(ProxyRequestIdempotencyBeginOutcome::Replay { record });
                    }
                    ProxyRequestIdempotencyState::InFlight => {
                        tx.commit().await?;
                        return Ok(ProxyRequestIdempotencyBeginOutcome::InFlight { record });
                    }
                }
            }
        }

        let record = new_proxy_request_idempotency_record(
            request_id.to_string(),
            fingerprint.clone(),
            fingerprint_key.to_string(),
            owner_token.to_string(),
            now_ms,
            lease_until_ms,
        );
        sqlx::query(
            "INSERT INTO proxy_request_idempotency (
                 request_id,
                 state,
                 owner_token,
                 lease_until_ms,
                 expires_at_ms,
                 updated_at_ms,
                 record_json
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (request_id) DO UPDATE SET
                 state = EXCLUDED.state,
                 owner_token = EXCLUDED.owner_token,
                 lease_until_ms = EXCLUDED.lease_until_ms,
                 expires_at_ms = EXCLUDED.expires_at_ms,
                 updated_at_ms = EXCLUDED.updated_at_ms,
                 record_json = EXCLUDED.record_json",
        )
        .bind(request_id)
        .bind(proxy_request_idempotency_state_label(record.state))
        .bind(owner_token)
        .bind(lease_until_ms_i64)
        .bind(lease_until_ms_i64)
        .bind(now_ms_i64)
        .bind(Json(&record))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(ProxyRequestIdempotencyBeginOutcome::Acquired)
    }

    pub async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, PostgresStoreError> {
        let row = sqlx::query(
            "SELECT record_json
             FROM proxy_request_idempotency
             WHERE request_id = $1 AND expires_at_ms >= $2",
        )
        .bind(request_id)
        .bind(u64_to_i64(now_ms))
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let Json(record): Json<ProxyRequestIdempotencyRecord> = row.try_get("record_json")?;
        Ok(Some(record))
    }

    pub async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, PostgresStoreError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT record_json
             FROM proxy_request_idempotency
             WHERE request_id = $1
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(false);
        };
        let Json(mut record): Json<ProxyRequestIdempotencyRecord> = row.try_get("record_json")?;
        if record.state != ProxyRequestIdempotencyState::InFlight
            || record.owner_token.as_deref() != Some(owner_token)
        {
            tx.commit().await?;
            return Ok(false);
        }

        let now_ms_i64 = u64_to_i64(now_ms);
        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        let lease_until_ms_i64 = u64_to_i64(lease_until_ms);
        record.updated_at_ms = now_ms;
        record.lease_until_ms = Some(lease_until_ms);
        record.expires_at_ms = lease_until_ms;

        let updated = sqlx::query(
            "UPDATE proxy_request_idempotency
             SET lease_until_ms = $3,
                 expires_at_ms = $4,
                 updated_at_ms = $5,
                 record_json = $6
             WHERE request_id = $1 AND owner_token = $2 AND state = $7",
        )
        .bind(request_id)
        .bind(owner_token)
        .bind(lease_until_ms_i64)
        .bind(lease_until_ms_i64)
        .bind(now_ms_i64)
        .bind(Json(&record))
        .bind(proxy_request_idempotency_state_label(
            ProxyRequestIdempotencyState::InFlight,
        ))
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;
        Ok(updated > 0)
    }

    pub async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, PostgresStoreError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT record_json
             FROM proxy_request_idempotency
             WHERE request_id = $1
             FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(false);
        };
        let Json(mut record): Json<ProxyRequestIdempotencyRecord> = row.try_get("record_json")?;
        if record.state != ProxyRequestIdempotencyState::InFlight
            || record.owner_token.as_deref() != Some(owner_token)
        {
            tx.commit().await?;
            return Ok(false);
        }

        let now_ms_i64 = u64_to_i64(now_ms);
        let expires_at_ms = now_ms.saturating_add(replay_ttl_ms);
        let expires_at_ms_i64 = u64_to_i64(expires_at_ms);
        record.state = ProxyRequestIdempotencyState::Completed;
        record.owner_token = None;
        record.lease_until_ms = None;
        record.completed_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        record.expires_at_ms = expires_at_ms;
        record.outcome = Some(outcome.clone());

        let updated = sqlx::query(
            "UPDATE proxy_request_idempotency
             SET state = $2,
                 owner_token = NULL,
                 lease_until_ms = NULL,
                 expires_at_ms = $3,
                 updated_at_ms = $4,
                 record_json = $5
             WHERE request_id = $1 AND owner_token = $6",
        )
        .bind(request_id)
        .bind(proxy_request_idempotency_state_label(record.state))
        .bind(expires_at_ms_i64)
        .bind(now_ms_i64)
        .bind(Json(&record))
        .bind(owner_token)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;
        Ok(updated > 0)
    }

    pub async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, PostgresStoreError> {
        let deleted = sqlx::query(
            "DELETE FROM proxy_request_idempotency
             WHERE request_id = $1 AND state = $2 AND owner_token = $3",
        )
        .bind(request_id)
        .bind(proxy_request_idempotency_state_label(
            ProxyRequestIdempotencyState::InFlight,
        ))
        .bind(owner_token)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deleted > 0)
    }
}

async fn ensure_pg_check_constraint(
    pool: &sqlx::PgPool,
    table: &str,
    constraint: &str,
    check_expr: &str,
) -> Result<(), PostgresStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM pg_catalog.pg_constraint c
         JOIN pg_catalog.pg_class t
           ON t.oid = c.conrelid
         JOIN pg_catalog.pg_namespace n
           ON n.oid = t.relnamespace
         WHERE c.contype = 'c'
           AND c.conname = $1
           AND t.relname = $2
           AND n.nspname = current_schema()
         LIMIT 1",
    )
    .bind(constraint)
    .bind(table)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        return Ok(());
    }

    let ddl = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} {check_expr}",
        quote_pg_ident(table),
        quote_pg_ident(constraint),
    );
    sqlx::query(&ddl).execute(pool).await?;
    Ok(())
}

async fn require_pg_table(pool: &sqlx::PgPool, table: &str) -> Result<(), PostgresStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM information_schema.tables
         WHERE table_schema = current_schema()
           AND table_name = $1
         LIMIT 1",
    )
    .bind(table)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        Ok(())
    } else {
        Err(PostgresStoreError::Schema(format!(
            "missing table `{table}` in current schema"
        )))
    }
}

async fn require_pg_column_udt(
    pool: &sqlx::PgPool,
    table: &str,
    column: &str,
    expected_udt: &str,
) -> Result<(), PostgresStoreError> {
    let row = sqlx::query(
        "SELECT udt_name
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = $1
           AND column_name = $2",
    )
    .bind(table)
    .bind(column)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(PostgresStoreError::Schema(format!(
            "missing column `{table}.{column}`"
        )));
    };

    let udt_name: String = row.try_get("udt_name")?;
    if udt_name.eq_ignore_ascii_case(expected_udt) {
        Ok(())
    } else {
        Err(PostgresStoreError::Schema(format!(
            "column `{table}.{column}` has type `{udt_name}`, expected `{expected_udt}`"
        )))
    }
}

async fn require_pg_index(
    pool: &sqlx::PgPool,
    table: &str,
    index: &str,
) -> Result<(), PostgresStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM pg_catalog.pg_class idx
         JOIN pg_catalog.pg_index i
           ON i.indexrelid = idx.oid
         JOIN pg_catalog.pg_class tbl
           ON tbl.oid = i.indrelid
         JOIN pg_catalog.pg_namespace ns
           ON ns.oid = tbl.relnamespace
         WHERE ns.nspname = current_schema()
           AND tbl.relname = $1
           AND idx.relname = $2
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
        Err(PostgresStoreError::Schema(format!(
            "missing index `{index}` on table `{table}`"
        )))
    }
}

async fn require_pg_check_constraint(
    pool: &sqlx::PgPool,
    table: &str,
    constraint: &str,
) -> Result<(), PostgresStoreError> {
    let exists = sqlx::query(
        "SELECT 1
         FROM pg_catalog.pg_constraint c
         JOIN pg_catalog.pg_class t
           ON t.oid = c.conrelid
         JOIN pg_catalog.pg_namespace n
           ON n.oid = t.relnamespace
         WHERE c.contype = 'c'
           AND c.conname = $1
           AND t.relname = $2
           AND n.nspname = current_schema()
         LIMIT 1",
    )
    .bind(constraint)
    .bind(table)
    .fetch_optional(pool)
    .await?
    .is_some();

    if exists {
        Ok(())
    } else {
        Err(PostgresStoreError::Schema(format!(
            "missing check constraint `{constraint}` on table `{table}`"
        )))
    }
}

fn quote_pg_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
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

    fn postgres_url() -> Option<String> {
        env_nonempty("DITTO_POSTGRES_URL").or_else(|| env_nonempty("POSTGRES_URL"))
    }

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_suffix() -> String {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", now_millis_i64())
    }

    #[tokio::test]
    async fn postgres_store_round_trips_config_audit_and_ledgers() {
        let Some(url) = postgres_url() else {
            eprintln!("skipping postgres test: set DITTO_POSTGRES_URL or POSTGRES_URL");
            return;
        };

        let store = PostgresStore::connect(url).await.expect("connect");
        store.ping().await.expect("ping");
        store.init().await.expect("init");
        store.verify_schema().await.expect("verify schema");

        let suffix = test_suffix();
        let key_id = format!("pg-key-{suffix}");
        let token = format!("pg-token-{suffix}");
        let request_id_budget = format!("pg-budget-{suffix}");
        let request_id_cost = format!("pg-cost-{suffix}");
        let request_id_reap_budget = format!("pg-reap-budget-{suffix}");
        let request_id_reap_cost = format!("pg-reap-cost-{suffix}");

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
            .append_audit_log("postgres.test", serde_json::json!({ "suffix": suffix }))
            .await
            .expect("append audit log");
        let logs = store
            .list_audit_logs(50, None)
            .await
            .expect("list audit logs");
        assert!(logs.iter().any(|record| record.kind == "postgres.test"));

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
