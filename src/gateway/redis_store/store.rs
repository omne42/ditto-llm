use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use redis::AsyncCommands;
use thiserror::Error;

use super::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, RouterConfig, VirtualKeyConfig};

#[cfg(feature = "gateway-proxy-cache")]
use super::{
    CachedProxyResponse, ProxyCacheEntryMetadata, ProxyCachePurgeSelector,
    ProxyCacheStoredResponse,
};
#[cfg(feature = "gateway-proxy-cache")]
use axum::http::{HeaderMap, HeaderName, HeaderValue};
#[cfg(feature = "gateway-proxy-cache")]
use bytes::Bytes;
#[cfg(feature = "gateway-proxy-cache")]
use serde::{Deserialize, Serialize};

// NOTE: We intentionally do not let reservation keys silently expire in Redis.
//
// Expiring the reservation hash would drop the only source of truth needed to
// reconcile `reserved_*` in the ledger, which can permanently lock budgets.
// Stale reservations should be reaped explicitly (e.g. via an admin
// maintenance endpoint) based on their stored `ts_ms`.
const DEFAULT_RESERVATION_TTL_SECS: u64 = 0;
const AUDIT_RETENTION_REAP_INTERVAL_MS: i64 = 30_000;

#[derive(Clone, Debug)]
pub struct RedisStore {
    client: redis::Client,
    prefix: String,
    audit_retention_secs: Option<u64>,
    audit_last_retention_reap_ms: Arc<AtomicI64>,
}

#[derive(Debug, Error)]
pub enum RedisStoreError {
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
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

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedProxyResponseRecord {
    status: u16,
    backend: String,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
    #[serde(default)]
    metadata: ProxyCacheEntryMetadata,
}

#[cfg(feature = "gateway-proxy-cache")]
impl CachedProxyResponseRecord {
    fn from_cached(cached: &CachedProxyResponse, metadata: &ProxyCacheEntryMetadata) -> Self {
        let mut headers = Vec::with_capacity(cached.headers.len());
        for (name, value) in cached.headers.iter() {
            headers.push((name.as_str().to_string(), value.as_bytes().to_vec()));
        }

        Self {
            status: cached.status,
            backend: cached.backend.clone(),
            headers,
            body: cached.body.as_ref().to_vec(),
            metadata: metadata.clone(),
        }
    }

    fn into_stored(self) -> ProxyCacheStoredResponse {
        let mut headers = HeaderMap::new();
        for (name, value) in self.headers {
            let Ok(name) = name.parse::<HeaderName>() else {
                continue;
            };
            let Ok(value) = HeaderValue::from_bytes(&value) else {
                continue;
            };
            headers.append(name, value);
        }

        ProxyCacheStoredResponse {
            response: CachedProxyResponse {
                status: self.status,
                headers,
                body: Bytes::from(self.body),
                backend: self.backend,
            },
            metadata: self.metadata,
        }
    }
}

impl RedisStore {
    pub fn new(url: impl AsRef<str>) -> Result<Self, RedisStoreError> {
        Ok(Self {
            client: redis::Client::open(url.as_ref())?,
            prefix: "ditto".to_string(),
            audit_retention_secs: None,
            audit_last_retention_reap_ms: Arc::new(AtomicI64::new(0)),
        })
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn with_audit_retention_secs(mut self, secs: Option<u64>) -> Self {
        self.audit_retention_secs = secs.filter(|value| *value > 0);
        self
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        self.client.get_multiplexed_async_connection().await
    }

    pub async fn ping(&self) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let _: Option<String> = conn.get(format!("{}:__ping__", self.prefix)).await?;
        Ok(())
    }

    fn key_virtual_keys(&self) -> String {
        format!("{}:virtual_keys", self.prefix)
    }

    fn key_router_config(&self) -> String {
        format!("{}:router_config", self.prefix)
    }

    fn key_budget_keys(&self) -> String {
        format!("{}:budget_keys", self.prefix)
    }

    fn key_budget_ledger(&self, key_id: &str) -> String {
        format!("{}:budget_ledger:{key_id}", self.prefix)
    }

    fn key_budget_reservation(&self, request_id: &str) -> String {
        format!("{}:budget_reservation:{request_id}", self.prefix)
    }

    fn key_cost_keys(&self) -> String {
        format!("{}:cost_keys", self.prefix)
    }

    fn key_cost_ledger(&self, key_id: &str) -> String {
        format!("{}:cost_ledger:{key_id}", self.prefix)
    }

    fn key_cost_reservation(&self, request_id: &str) -> String {
        format!("{}:cost_reservation:{request_id}", self.prefix)
    }

    fn key_audit_seq(&self) -> String {
        format!("{}:audit_seq", self.prefix)
    }

    fn key_audit_by_ts(&self) -> String {
        format!("{}:audit_by_ts", self.prefix)
    }

    fn key_audit_record(&self, id: &str) -> String {
        format!("{}:audit:{id}", self.prefix)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    fn key_proxy_cache_response(&self, cache_key: &str) -> String {
        format!("{}:proxy_cache:{cache_key}", self.prefix)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    fn proxy_cache_pattern(&self) -> String {
        format!("{}:proxy_cache:*", self.prefix)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    fn proxy_cache_key_from_redis_key<'a>(&self, redis_key: &'a str) -> Option<&'a str> {
        let prefix = format!("{}:proxy_cache:", self.prefix);
        redis_key.strip_prefix(&prefix)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    fn record_matches_selector(
        &self,
        selector: &ProxyCachePurgeSelector,
        cache_key: &str,
        raw: &[u8],
    ) -> bool {
        let Ok(record) = serde_json::from_slice::<CachedProxyResponseRecord>(raw) else {
            return false;
        };
        selector.matches(cache_key, &record.metadata)
    }
}

fn audit_cutoff_ms(retention_secs: Option<u64>, now_ms: u64) -> Option<u64> {
    let retention_secs = retention_secs?;
    let retention_ms = retention_secs.saturating_mul(1000);
    Some(now_ms.saturating_sub(retention_ms))
}

fn should_run_retention_reap(last_reap_ms: &AtomicI64, now_ms: i64) -> bool {
    let mut observed = last_reap_ms.load(Ordering::Relaxed);
    loop {
        if observed > 0
            && now_ms.saturating_sub(observed) < AUDIT_RETENTION_REAP_INTERVAL_MS
        {
            return false;
        }
        match last_reap_ms.compare_exchange(
            observed,
            now_ms,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => observed = actual,
        }
    }
}
