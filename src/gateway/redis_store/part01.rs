use std::collections::HashMap;

use redis::AsyncCommands;
use thiserror::Error;

use super::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, VirtualKeyConfig};

#[cfg(feature = "gateway-proxy-cache")]
use super::CachedProxyResponse;
#[cfg(feature = "gateway-proxy-cache")]
use axum::http::{HeaderMap, HeaderName, HeaderValue};
#[cfg(feature = "gateway-proxy-cache")]
use bytes::Bytes;
#[cfg(feature = "gateway-proxy-cache")]
use serde::{Deserialize, Serialize};

const DEFAULT_RESERVATION_TTL_SECS: u64 = 60 * 60;

#[derive(Clone, Debug)]
pub struct RedisStore {
    client: redis::Client,
    prefix: String,
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
}

#[cfg(feature = "gateway-proxy-cache")]
impl CachedProxyResponseRecord {
    fn from_cached(cached: &CachedProxyResponse) -> Self {
        let mut headers = Vec::with_capacity(cached.headers.len());
        for (name, value) in cached.headers.iter() {
            headers.push((name.as_str().to_string(), value.as_bytes().to_vec()));
        }

        Self {
            status: cached.status,
            backend: cached.backend.clone(),
            headers,
            body: cached.body.as_ref().to_vec(),
        }
    }

    fn into_cached(self) -> CachedProxyResponse {
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

        CachedProxyResponse {
            status: self.status,
            headers,
            body: Bytes::from(self.body),
            backend: self.backend,
        }
    }
}

impl RedisStore {
    pub fn new(url: impl AsRef<str>) -> Result<Self, RedisStoreError> {
        Ok(Self {
            client: redis::Client::open(url.as_ref())?,
            prefix: "ditto".to_string(),
        })
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
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
}
