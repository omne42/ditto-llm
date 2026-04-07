// Gateway Redis adapter implementation.
// inlined from ../../../redis_store/store.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use redis::AsyncCommands;
use thiserror::Error;

use super::{
    AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, ProxyRequestFingerprint,
    ProxyRequestIdempotencyBeginOutcome, ProxyRequestIdempotencyRecord,
    ProxyRequestIdempotencyState, ProxyRequestReplayOutcome, RouterConfig, VirtualKeyConfig,
};

#[cfg(feature = "gateway-proxy-cache")]
use super::{
    CachedProxyResponse, ProxyCacheEntryMetadata, ProxyCachePurgeSelector, ProxyCacheStoredResponse,
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
const BEGIN_PROXY_REQUEST_IDEMPOTENCY_SCRIPT: &str = r#"
local key = KEYS[1]
local now_ms = tonumber(ARGV[1])
local lease_ttl_ms = tonumber(ARGV[2])
local record_json = ARGV[3]
local fingerprint_key = ARGV[4]

local raw = redis.call('GET', key)
if not raw then
  redis.call('PSETEX', key, lease_ttl_ms, record_json)
  return { 'acquired' }
end

local ok, record = pcall(cjson.decode, raw)
if not ok then
  redis.call('PSETEX', key, lease_ttl_ms, record_json)
  return { 'acquired' }
end

local expires_at_ms = tonumber(record['expires_at_ms'] or '0')
if expires_at_ms < now_ms then
  redis.call('PSETEX', key, lease_ttl_ms, record_json)
  return { 'acquired' }
end

if record['fingerprint_key'] ~= fingerprint_key then
  return { 'conflict', raw }
end

if record['state'] == 'completed' then
  return { 'replay', raw }
end

return { 'in_flight', raw }
"#;

const COMPLETE_PROXY_REQUEST_IDEMPOTENCY_SCRIPT: &str = r#"
local key = KEYS[1]
local owner_token = ARGV[1]
local replay_ttl_ms = tonumber(ARGV[2])
local record_json = ARGV[3]

local raw = redis.call('GET', key)
if not raw then
  return 0
end

local ok, record = pcall(cjson.decode, raw)
if not ok then
  return 0
end

if record['state'] ~= 'in_flight' then
  return 0
end

if record['owner_token'] ~= owner_token then
  return 0
end

redis.call('PSETEX', key, replay_ttl_ms, record_json)
return 1
"#;

const REFRESH_PROXY_REQUEST_IDEMPOTENCY_SCRIPT: &str = r#"
local key = KEYS[1]
local owner_token = ARGV[1]
local lease_ttl_ms = tonumber(ARGV[2])
local record_json = ARGV[3]

local raw = redis.call('GET', key)
if not raw then
  return 0
end

local ok, record = pcall(cjson.decode, raw)
if not ok then
  return 0
end

if record['state'] ~= 'in_flight' then
  return 0
end

if record['owner_token'] ~= owner_token then
  return 0
end

redis.call('PSETEX', key, lease_ttl_ms, record_json)
return 1
"#;

const RELEASE_PROXY_REQUEST_IDEMPOTENCY_SCRIPT: &str = r#"
local key = KEYS[1]
local owner_token = ARGV[1]

local raw = redis.call('GET', key)
if not raw then
  return 0
end

local ok, record = pcall(cjson.decode, raw)
if not ok then
  return 0
end

if record['state'] ~= 'in_flight' then
  return 0
end

if record['owner_token'] ~= owner_token then
  return 0
end

redis.call('DEL', key)
return 1
"#;

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
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("integer parse error: {0}")]
    ParseInt(#[from] std::num::ParseIntError),
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

    fn key_proxy_request_idempotency(&self, request_id: &str) -> String {
        format!("{}:proxy_request_idempotency:{request_id}", self.prefix)
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
        if observed > 0 && now_ms.saturating_sub(observed) < AUDIT_RETENTION_REAP_INTERVAL_MS {
            return false;
        }
        match last_reap_ms.compare_exchange(observed, now_ms, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => observed = actual,
        }
    }
}

// end inline: ../../../redis_store/store.rs
// inlined from ../../../redis_store/virtual_keys_and_proxy_cache.rs
impl RedisStore {
    pub async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyConfig>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let key = self.key_virtual_keys();
        let raw_map: HashMap<String, String> = conn.hgetall(key).await?;
        let mut out: Vec<VirtualKeyConfig> = Vec::with_capacity(raw_map.len());
        for (_id, raw) in raw_map {
            out.push(serde_json::from_str(&raw)?);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub async fn replace_virtual_keys(
        &self,
        keys: &[VirtualKeyConfig],
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_virtual_keys();

        let mut pipe = redis::pipe();
        pipe.atomic().del(&redis_key);
        for key in keys {
            let key = key.sanitized_for_persistence();
            pipe.hset(&redis_key, &key.id, serde_json::to_string(&key)?);
        }
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn load_router_config(&self) -> Result<Option<RouterConfig>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_router_config();
        let raw: Option<String> = conn.get(redis_key).await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub async fn replace_router_config(
        &self,
        router: &RouterConfig,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_router_config();
        let _: () = conn.set(redis_key, serde_json::to_string(router)?).await?;
        Ok(())
    }

    pub async fn replace_control_plane_snapshot(
        &self,
        keys: &[VirtualKeyConfig],
        router: &RouterConfig,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let virtual_keys_key = self.key_virtual_keys();
        let router_key = self.key_router_config();
        let router_json = serde_json::to_string(router)?;

        let mut pipe = redis::pipe();
        pipe.atomic().del(&virtual_keys_key);
        for key in keys {
            let key = key.sanitized_for_persistence();
            pipe.hset(&virtual_keys_key, &key.id, serde_json::to_string(&key)?);
        }
        pipe.set(&router_key, router_json);
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn begin_proxy_request_idempotency(
        &self,
        request_id: &str,
        fingerprint: &ProxyRequestFingerprint,
        fingerprint_key: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<ProxyRequestIdempotencyBeginOutcome, RedisStoreError> {
        let mut conn = self.connection().await?;
        let record = ProxyRequestIdempotencyRecord {
            request_id: request_id.to_string(),
            fingerprint: fingerprint.clone(),
            fingerprint_key: fingerprint_key.to_string(),
            state: ProxyRequestIdempotencyState::InFlight,
            owner_token: Some(owner_token.to_string()),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
            lease_until_ms: Some(now_ms.saturating_add(lease_ttl_ms)),
            completed_at_ms: None,
            expires_at_ms: now_ms.saturating_add(lease_ttl_ms),
            outcome: None,
        };
        let payload = serde_json::to_string(&record)?;
        let script = redis::Script::new(BEGIN_PROXY_REQUEST_IDEMPOTENCY_SCRIPT);
        let result: Vec<String> = script
            .key(self.key_proxy_request_idempotency(request_id))
            .arg(now_ms.to_string())
            .arg(lease_ttl_ms.max(1).to_string())
            .arg(payload)
            .arg(fingerprint_key)
            .invoke_async(&mut conn)
            .await?;
        match result.first().map(String::as_str) {
            Some("acquired") => Ok(ProxyRequestIdempotencyBeginOutcome::Acquired),
            Some("replay") => Ok(ProxyRequestIdempotencyBeginOutcome::Replay {
                record: serde_json::from_str(
                    result.get(1).map(String::as_str).unwrap_or_default(),
                )?,
            }),
            Some("in_flight") => Ok(ProxyRequestIdempotencyBeginOutcome::InFlight {
                record: serde_json::from_str(
                    result.get(1).map(String::as_str).unwrap_or_default(),
                )?,
            }),
            Some("conflict") => Ok(ProxyRequestIdempotencyBeginOutcome::Conflict {
                record: serde_json::from_str(
                    result.get(1).map(String::as_str).unwrap_or_default(),
                )?,
            }),
            _ => Ok(ProxyRequestIdempotencyBeginOutcome::Acquired),
        }
    }

    pub async fn get_proxy_request_idempotency(
        &self,
        request_id: &str,
        _now_ms: u64,
    ) -> Result<Option<ProxyRequestIdempotencyRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let raw: Option<String> = conn
            .get(self.key_proxy_request_idempotency(request_id))
            .await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub async fn refresh_proxy_request_idempotency_lease(
        &self,
        request_id: &str,
        owner_token: &str,
        now_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<bool, RedisStoreError> {
        let mut conn = self.connection().await?;
        let key = self.key_proxy_request_idempotency(request_id);
        let raw: Option<String> = conn.get(&key).await?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let mut record: ProxyRequestIdempotencyRecord = serde_json::from_str(&raw)?;
        if record.owner_token.as_deref() != Some(owner_token)
            || !matches!(record.state, super::ProxyRequestIdempotencyState::InFlight)
        {
            return Ok(false);
        }

        let lease_until_ms = now_ms.saturating_add(lease_ttl_ms);
        record.updated_at_ms = now_ms;
        record.lease_until_ms = Some(lease_until_ms);
        record.expires_at_ms = lease_until_ms;
        let payload = serde_json::to_string(&record)?;
        let updated: i64 = redis::Script::new(REFRESH_PROXY_REQUEST_IDEMPOTENCY_SCRIPT)
            .key(key)
            .arg(owner_token)
            .arg(lease_ttl_ms.max(1).to_string())
            .arg(payload)
            .invoke_async(&mut conn)
            .await?;
        Ok(updated > 0)
    }

    pub async fn complete_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
        outcome: &ProxyRequestReplayOutcome,
        now_ms: u64,
        replay_ttl_ms: u64,
    ) -> Result<bool, RedisStoreError> {
        let mut conn = self.connection().await?;
        let key = self.key_proxy_request_idempotency(request_id);
        let raw: Option<String> = conn.get(&key).await?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let mut record: ProxyRequestIdempotencyRecord = serde_json::from_str(&raw)?;
        if record.owner_token.as_deref() != Some(owner_token)
            || !matches!(record.state, ProxyRequestIdempotencyState::InFlight)
        {
            return Ok(false);
        }
        record.state = ProxyRequestIdempotencyState::Completed;
        record.owner_token = None;
        record.lease_until_ms = None;
        record.completed_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        record.expires_at_ms = now_ms.saturating_add(replay_ttl_ms);
        record.outcome = Some(outcome.clone());
        let payload = serde_json::to_string(&record)?;
        let updated: i64 = redis::Script::new(COMPLETE_PROXY_REQUEST_IDEMPOTENCY_SCRIPT)
            .key(key)
            .arg(owner_token)
            .arg(replay_ttl_ms.max(1).to_string())
            .arg(payload)
            .invoke_async(&mut conn)
            .await?;
        Ok(updated > 0)
    }

    pub async fn release_proxy_request_idempotency(
        &self,
        request_id: &str,
        owner_token: &str,
    ) -> Result<bool, RedisStoreError> {
        let mut conn = self.connection().await?;
        let deleted: i64 = redis::Script::new(RELEASE_PROXY_REQUEST_IDEMPOTENCY_SCRIPT)
            .key(self.key_proxy_request_idempotency(request_id))
            .arg(owner_token)
            .invoke_async(&mut conn)
            .await?;
        Ok(deleted > 0)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn get_proxy_cache_response(
        &self,
        cache_key: &str,
    ) -> Result<Option<ProxyCacheStoredResponse>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let raw: Option<Vec<u8>> = conn.get(redis_key).await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let record: CachedProxyResponseRecord = match serde_json::from_slice(&raw) {
            Ok(record) => record,
            Err(_) => return Ok(None),
        };
        Ok(Some(record.into_stored()))
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn set_proxy_cache_response(
        &self,
        cache_key: &str,
        cached: &CachedProxyResponse,
        metadata: &ProxyCacheEntryMetadata,
        ttl_seconds: u64,
    ) -> Result<(), RedisStoreError> {
        if ttl_seconds == 0 {
            return Ok(());
        }

        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let payload =
            serde_json::to_vec(&CachedProxyResponseRecord::from_cached(cached, metadata))?;
        let _: () = conn.set_ex(redis_key, payload, ttl_seconds).await?;
        Ok(())
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn delete_proxy_cache_response(
        &self,
        cache_key: &str,
    ) -> Result<u64, RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let deleted: u64 = conn.del(redis_key).await?;
        Ok(deleted)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn purge_proxy_cache_matching(
        &self,
        selector: &ProxyCachePurgeSelector,
    ) -> Result<u64, RedisStoreError> {
        let selector = selector.clone().into_normalized();
        if selector.is_empty() {
            return Ok(0);
        }

        if let Some(cache_key) = selector.cache_key.as_deref() {
            let mut conn = self.connection().await?;
            let redis_key = self.key_proxy_cache_response(cache_key);
            if selector.as_exact_cache_key().is_some() {
                let deleted: u64 = conn.del(redis_key).await?;
                return Ok(deleted);
            }

            let raw: Option<Vec<u8>> = conn.get(&redis_key).await?;
            let Some(raw) = raw else {
                return Ok(0);
            };
            if self.record_matches_selector(&selector, cache_key, &raw) {
                let deleted: u64 = conn.del(redis_key).await?;
                return Ok(deleted);
            }
            return Ok(0);
        }

        let pattern = self.proxy_cache_pattern();
        let mut conn = self.connection().await?;
        let mut deleted = 0u64;
        let mut cursor = "0".to_string();

        loop {
            let (next_cursor, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(&cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await?;

            for redis_key in keys {
                let Some(cache_key) = self.proxy_cache_key_from_redis_key(&redis_key) else {
                    continue;
                };
                let raw: Option<Vec<u8>> = conn.get(&redis_key).await?;
                let Some(raw) = raw else {
                    continue;
                };
                if self.record_matches_selector(&selector, cache_key, &raw) {
                    let removed: u64 = conn.del(&redis_key).await?;
                    deleted = deleted.saturating_add(removed);
                }
            }

            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }
        Ok(deleted)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn clear_proxy_cache(&self) -> Result<u64, RedisStoreError> {
        let pattern = self.proxy_cache_pattern();
        let mut conn = self.connection().await?;
        let mut deleted = 0u64;

        let mut cursor = "0".to_string();
        loop {
            let (next_cursor, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(&cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await?;

            for chunk in keys.chunks(128) {
                deleted = deleted.saturating_add(conn.del(chunk).await?);
            }

            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }
        Ok(deleted)
    }
}
// end inline: ../../../redis_store/virtual_keys_and_proxy_cache.rs
// inlined from ../../../redis_store/budget.rs
#[cfg(feature = "gateway-store-redis")]
const REAP_BUDGET_RESERVATION_SCRIPT: &str = r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local cutoff_ts_ms = tonumber(ARGV[2]) or 0
local ts_ms = ARGV[3]
local dry_run = tonumber(ARGV[4]) or 0

if redis.call("EXISTS", reservation_key) == 0 then
  return { "MISS" }
end

local reservation_ts = tonumber(redis.call("HGET", reservation_key, "ts_ms") or "0") or 0
if reservation_ts > cutoff_ts_ms then
  return { "KEEP" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local tokens = tonumber(redis.call("HGET", reservation_key, "tokens") or "0") or 0

if (not key_id) then
  return { "INVALID" }
end

if dry_run == 1 then
  return { "DRY", tostring(tokens) }
end

redis.call("DEL", reservation_key)

local ledger_key = prefix .. ":budget_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_tokens", -tokens) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_tokens", 0)
end

redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "REAP", tostring(tokens) }
"#;

#[cfg(feature = "gateway-store-redis")]
const REAP_COST_RESERVATION_SCRIPT: &str = r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local cutoff_ts_ms = tonumber(ARGV[2]) or 0
local ts_ms = ARGV[3]
local dry_run = tonumber(ARGV[4]) or 0

if redis.call("EXISTS", reservation_key) == 0 then
  return { "MISS" }
end

local reservation_ts = tonumber(redis.call("HGET", reservation_key, "ts_ms") or "0") or 0
if reservation_ts > cutoff_ts_ms then
  return { "KEEP" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local usd_micros = tonumber(redis.call("HGET", reservation_key, "usd_micros") or "0") or 0

if (not key_id) then
  return { "INVALID" }
end

if dry_run == 1 then
  return { "DRY", tostring(usd_micros) }
end

redis.call("DEL", reservation_key)

local ledger_key = prefix .. ":cost_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_usd_micros", -usd_micros) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_usd_micros", 0)
end

redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "REAP", tostring(usd_micros) }
"#;

impl RedisStore {
    pub async fn reserve_budget_tokens(
        &self,
        request_id: &str,
        key_id: &str,
        limit: u64,
        tokens: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let ts_ms = now_millis();

        let reservation_key = self.key_budget_reservation(request_id);
        let ledger_key = self.key_budget_ledger(key_id);
        let keys_key = self.key_budget_keys();

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]
local ledger_key = KEYS[3]

local key_id = ARGV[1]
local tokens = tonumber(ARGV[2]) or 0
local limit = tonumber(ARGV[3]) or 0
local ts_ms = ARGV[4]
local ttl_secs = tonumber(ARGV[5]) or 0

if redis.call("EXISTS", reservation_key) == 1 then
  return { "OK", "exists" }
end

local spent = tonumber(redis.call("HGET", ledger_key, "spent_tokens") or "0") or 0
local reserved = tonumber(redis.call("HGET", ledger_key, "reserved_tokens") or "0") or 0
local attempted = spent + reserved + tokens
if attempted > limit then
  return { "ERR", "budget_exceeded", tostring(attempted) }
end

redis.call("HSET", reservation_key, "key_id", key_id, "tokens", tostring(tokens), "ts_ms", ts_ms)
if ttl_secs > 0 then
  redis.call("EXPIRE", reservation_key, ttl_secs)
end

redis.call("HINCRBY", ledger_key, "reserved_tokens", tokens)
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK" }
"#,
        );

        let result: Vec<String> = script
            .key(keys_key)
            .key(reservation_key)
            .key(ledger_key)
            .arg(key_id)
            .arg(tokens)
            .arg(limit)
            .arg(ts_ms)
            .arg(DEFAULT_RESERVATION_TTL_SECS)
            .invoke_async(&mut conn)
            .await?;

        match result.first().map(|s| s.as_str()) {
            Some("OK") => Ok(()),
            Some("ERR") if result.get(1).map(|s| s.as_str()) == Some("budget_exceeded") => {
                let attempted = result
                    .get(2)
                    .and_then(|raw| raw.parse::<u64>().ok())
                    .unwrap_or_else(|| limit.saturating_add(tokens));
                Err(RedisStoreError::BudgetExceeded { limit, attempted })
            }
            _ => Err(redis::RedisError::from((
                redis::ErrorKind::ResponseError,
                "unexpected redis script response",
            ))
            .into()),
        }
    }

    pub async fn reserve_cost_usd_micros(
        &self,
        request_id: &str,
        key_id: &str,
        limit_usd_micros: u64,
        usd_micros: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let ts_ms = now_millis();

        let reservation_key = self.key_cost_reservation(request_id);
        let ledger_key = self.key_cost_ledger(key_id);
        let keys_key = self.key_cost_keys();

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]
local ledger_key = KEYS[3]

local key_id = ARGV[1]
local usd_micros = tonumber(ARGV[2]) or 0
local limit = tonumber(ARGV[3]) or 0
local ts_ms = ARGV[4]
local ttl_secs = tonumber(ARGV[5]) or 0

if redis.call("EXISTS", reservation_key) == 1 then
  return { "OK", "exists" }
end

local spent = tonumber(redis.call("HGET", ledger_key, "spent_usd_micros") or "0") or 0
local reserved = tonumber(redis.call("HGET", ledger_key, "reserved_usd_micros") or "0") or 0
local attempted = spent + reserved + usd_micros
if attempted > limit then
  return { "ERR", "cost_budget_exceeded", tostring(attempted) }
end

redis.call("HSET", reservation_key, "key_id", key_id, "usd_micros", tostring(usd_micros), "ts_ms", ts_ms)
if ttl_secs > 0 then
  redis.call("EXPIRE", reservation_key, ttl_secs)
end

redis.call("HINCRBY", ledger_key, "reserved_usd_micros", usd_micros)
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK" }
"#,
        );

        let result: Vec<String> = script
            .key(keys_key)
            .key(reservation_key)
            .key(ledger_key)
            .arg(key_id)
            .arg(usd_micros)
            .arg(limit_usd_micros)
            .arg(ts_ms)
            .arg(DEFAULT_RESERVATION_TTL_SECS)
            .invoke_async(&mut conn)
            .await?;

        match result.first().map(|s| s.as_str()) {
            Some("OK") => Ok(()),
            Some("ERR") if result.get(1).map(|s| s.as_str()) == Some("cost_budget_exceeded") => {
                let attempted = result
                    .get(2)
                    .and_then(|raw| raw.parse::<u64>().ok())
                    .unwrap_or_else(|| limit_usd_micros.saturating_add(usd_micros));
                Err(RedisStoreError::CostBudgetExceeded {
                    limit_usd_micros,
                    attempted_usd_micros: attempted,
                })
            }
            _ => Err(redis::RedisError::from((
                redis::ErrorKind::ResponseError,
                "unexpected redis script response",
            ))
            .into()),
        }
    }

    pub async fn commit_budget_reservation(&self, request_id: &str) -> Result<(), RedisStoreError> {
        self.commit_budget_reservation_with_tokens(request_id, u64::MAX)
            .await
    }

    pub async fn commit_budget_reservation_with_tokens(
        &self,
        request_id: &str,
        spent_tokens: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let reservation_key = self.key_budget_reservation(request_id);
        let ts_ms = now_millis();
        let spent_tokens = tokens_to_i64(spent_tokens);

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local spent_tokens = tonumber(ARGV[2]) or 0
local ts_ms = ARGV[3]

if redis.call("EXISTS", reservation_key) == 0 then
  return { "OK", "missing" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local reserved_tokens = tonumber(redis.call("HGET", reservation_key, "tokens") or "0") or 0
redis.call("DEL", reservation_key)
if (not key_id) then
  return { "OK", "missing_key" }
end

if spent_tokens == 9223372036854775807 then
  spent_tokens = reserved_tokens
end

local ledger_key = prefix .. ":budget_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_tokens", -reserved_tokens) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_tokens", 0)
end
redis.call("HINCRBY", ledger_key, "spent_tokens", spent_tokens)
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK", key_id }
"#,
        );

        let _: Vec<String> = script
            .key(self.key_budget_keys())
            .key(reservation_key)
            .arg(self.prefix.clone())
            .arg(spent_tokens)
            .arg(ts_ms)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn commit_cost_reservation(&self, request_id: &str) -> Result<(), RedisStoreError> {
        self.commit_cost_reservation_with_usd_micros(request_id, u64::MAX)
            .await
    }

    pub async fn commit_cost_reservation_with_usd_micros(
        &self,
        request_id: &str,
        spent_usd_micros: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let reservation_key = self.key_cost_reservation(request_id);
        let ts_ms = now_millis();
        let spent_usd_micros = tokens_to_i64(spent_usd_micros);

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local spent_usd_micros = tonumber(ARGV[2]) or 0
local ts_ms = ARGV[3]

if redis.call("EXISTS", reservation_key) == 0 then
  return { "OK", "missing" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local reserved_usd_micros = tonumber(redis.call("HGET", reservation_key, "usd_micros") or "0") or 0
redis.call("DEL", reservation_key)
if (not key_id) then
  return { "OK", "missing_key" }
end

if spent_usd_micros == 9223372036854775807 then
  spent_usd_micros = reserved_usd_micros
end

local ledger_key = prefix .. ":cost_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_usd_micros", -reserved_usd_micros) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_usd_micros", 0)
end
redis.call("HINCRBY", ledger_key, "spent_usd_micros", spent_usd_micros)
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK", key_id }
"#,
        );

        let _: Vec<String> = script
            .key(self.key_cost_keys())
            .key(reservation_key)
            .arg(self.prefix.clone())
            .arg(spent_usd_micros)
            .arg(ts_ms)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn rollback_budget_reservation(
        &self,
        request_id: &str,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let reservation_key = self.key_budget_reservation(request_id);
        let ts_ms = now_millis();

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local ts_ms = ARGV[2]

if redis.call("EXISTS", reservation_key) == 0 then
  return { "OK", "missing" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local tokens = tonumber(redis.call("HGET", reservation_key, "tokens") or "0") or 0
redis.call("DEL", reservation_key)
if (not key_id) then
  return { "OK", "missing_key" }
end

local ledger_key = prefix .. ":budget_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_tokens", -tokens) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_tokens", 0)
end
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK", key_id }
"#,
        );

        let _: Vec<String> = script
            .key(self.key_budget_keys())
            .key(reservation_key)
            .arg(self.prefix.clone())
            .arg(ts_ms)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn rollback_cost_reservation(&self, request_id: &str) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let reservation_key = self.key_cost_reservation(request_id);
        let ts_ms = now_millis();

        let script = redis::Script::new(
            r#"
local keys_key = KEYS[1]
local reservation_key = KEYS[2]

local prefix = ARGV[1]
local ts_ms = ARGV[2]

if redis.call("EXISTS", reservation_key) == 0 then
  return { "OK", "missing" }
end

local key_id = redis.call("HGET", reservation_key, "key_id")
local usd_micros = tonumber(redis.call("HGET", reservation_key, "usd_micros") or "0") or 0
redis.call("DEL", reservation_key)
if (not key_id) then
  return { "OK", "missing_key" }
end

local ledger_key = prefix .. ":cost_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_usd_micros", -usd_micros) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_usd_micros", 0)
end
redis.call("HSET", ledger_key, "updated_at_ms", ts_ms)
redis.call("SADD", keys_key, key_id)
return { "OK", key_id }
"#,
        );

        let _: Vec<String> = script
            .key(self.key_cost_keys())
            .key(reservation_key)
            .arg(self.prefix.clone())
            .arg(ts_ms)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn record_spent_tokens(
        &self,
        key_id: &str,
        tokens: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let ledger_key = self.key_budget_ledger(key_id);
        let ts_ms = now_millis();
        let tokens = tokens_to_i64(tokens);
        let _: () = redis::pipe()
            .atomic()
            .hincr(&ledger_key, "spent_tokens", tokens)
            .hset(&ledger_key, "updated_at_ms", ts_ms)
            .sadd(self.key_budget_keys(), key_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn record_spent_cost_usd_micros(
        &self,
        key_id: &str,
        usd_micros: u64,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let ledger_key = self.key_cost_ledger(key_id);
        let ts_ms = now_millis();
        let usd_micros = tokens_to_i64(usd_micros);
        let _: () = redis::pipe()
            .atomic()
            .hincr(&ledger_key, "spent_usd_micros", usd_micros)
            .hset(&ledger_key, "updated_at_ms", ts_ms)
            .sadd(self.key_cost_keys(), key_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn list_budget_ledgers(&self) -> Result<Vec<BudgetLedgerRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let mut key_ids: Vec<String> = conn.smembers(self.key_budget_keys()).await?;
        key_ids.sort();

        let mut out = Vec::with_capacity(key_ids.len());
        for key_id in key_ids {
            let ledger_key = self.key_budget_ledger(&key_id);
            let raw: HashMap<String, String> = conn.hgetall(ledger_key).await?;
            let spent_tokens = raw
                .get("spent_tokens")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let reserved_tokens = raw
                .get("reserved_tokens")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let updated_at_ms = raw
                .get("updated_at_ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            out.push(BudgetLedgerRecord {
                key_id,
                spent_tokens,
                reserved_tokens,
                updated_at_ms,
            });
        }
        Ok(out)
    }

    pub async fn reap_stale_budget_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), RedisStoreError> {
        let max_reaped = max_reaped.clamp(1, 100_000);
        let mut conn = self.connection().await?;
        let pattern = format!("{}:budget_reservation:*", self.prefix);
        let ts_ms = now_millis();
        let dry_run = if dry_run { 1 } else { 0 };
        let script = redis::Script::new(REAP_BUDGET_RESERVATION_SCRIPT);

        let mut scanned = 0u64;
        let mut reaped = 0u64;
        let mut released_tokens = 0u64;

        let mut cursor = "0".to_string();
        loop {
            let (next_cursor, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(&cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await?;

            for reservation_key in keys {
                scanned = scanned.saturating_add(1);
                let result: Vec<String> = script
                    .key(self.key_budget_keys())
                    .key(reservation_key)
                    .arg(self.prefix.clone())
                    .arg(cutoff_ts_ms)
                    .arg(ts_ms)
                    .arg(dry_run)
                    .invoke_async(&mut conn)
                    .await?;

                match result.first().map(|s| s.as_str()) {
                    Some("REAP") | Some("DRY") => {
                        reaped = reaped.saturating_add(1);
                        released_tokens = released_tokens.saturating_add(
                            result
                                .get(1)
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or(0),
                        );
                    }
                    _ => {}
                }

                if reaped as usize >= max_reaped {
                    return Ok((scanned, reaped, released_tokens));
                }
            }

            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }

        Ok((scanned, reaped, released_tokens))
    }

    pub async fn reap_stale_cost_reservations(
        &self,
        cutoff_ts_ms: u64,
        max_reaped: usize,
        dry_run: bool,
    ) -> Result<(u64, u64, u64), RedisStoreError> {
        let max_reaped = max_reaped.clamp(1, 100_000);
        let mut conn = self.connection().await?;
        let pattern = format!("{}:cost_reservation:*", self.prefix);
        let ts_ms = now_millis();
        let dry_run = if dry_run { 1 } else { 0 };
        let script = redis::Script::new(REAP_COST_RESERVATION_SCRIPT);

        let mut scanned = 0u64;
        let mut reaped = 0u64;
        let mut released_usd_micros = 0u64;

        let mut cursor = "0".to_string();
        loop {
            let (next_cursor, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(&cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await?;

            for reservation_key in keys {
                scanned = scanned.saturating_add(1);
                let result: Vec<String> = script
                    .key(self.key_cost_keys())
                    .key(reservation_key)
                    .arg(self.prefix.clone())
                    .arg(cutoff_ts_ms)
                    .arg(ts_ms)
                    .arg(dry_run)
                    .invoke_async(&mut conn)
                    .await?;

                match result.first().map(|s| s.as_str()) {
                    Some("REAP") | Some("DRY") => {
                        reaped = reaped.saturating_add(1);
                        released_usd_micros = released_usd_micros.saturating_add(
                            result
                                .get(1)
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or(0),
                        );
                    }
                    _ => {}
                }

                if reaped as usize >= max_reaped {
                    return Ok((scanned, reaped, released_usd_micros));
                }
            }

            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }

        Ok((scanned, reaped, released_usd_micros))
    }
}
// end inline: ../../../redis_store/budget.rs
// inlined from ../../../redis_store/rate_limits.rs
#[cfg(feature = "gateway-store-redis")]
const DEFAULT_RATE_LIMIT_TTL_SECS: u64 = 3 * 60;

#[cfg(feature = "gateway-store-redis")]
const RATE_LIMIT_SCRIPT: &str = r#"
local rpm = tonumber(ARGV[1])
local tpm = tonumber(ARGV[2])
local tokens = tonumber(ARGV[3])
local second = tonumber(ARGV[4])
local ttl = tonumber(ARGV[5])

local req_cur = tonumber(redis.call("GET", KEYS[1]) or "0")
local req_prev = tonumber(redis.call("GET", KEYS[2]) or "0")
local tok_cur = tonumber(redis.call("GET", KEYS[3]) or "0")
local tok_prev = tonumber(redis.call("GET", KEYS[4]) or "0")

local next_req_cur = req_cur + 1
local next_tok_cur = tok_cur + tokens

local window_weight = 60 - second
local weighted_req = next_req_cur * 60 + req_prev * window_weight
local weighted_tok = next_tok_cur * 60 + tok_prev * window_weight

-- return codes:
--   1 = allowed
--   2 = blocked by rpm
--   3 = blocked by tpm
if rpm == 0 then
  return 2
end
if rpm ~= nil and rpm > 0 and weighted_req > (rpm * 60) then
  return 2
end

if tpm == 0 then
  return 3
end
if tpm ~= nil and tpm > 0 and weighted_tok > (tpm * 60) then
  return 3
end

redis.call("SET", KEYS[1], next_req_cur, "EX", ttl)
redis.call("SET", KEYS[3], next_tok_cur, "EX", ttl)
return 1
"#;

#[cfg(feature = "gateway-store-redis")]
const RATE_LIMIT_MANY_SCRIPT: &str = r#"
local scope_count = tonumber(ARGV[1])
local tokens = tonumber(ARGV[2])
local second = tonumber(ARGV[3])
local ttl = tonumber(ARGV[4])
local window_weight = 60 - second

local updates = {}
local arg_index = 5
for scope_index = 1, scope_count do
  local rpm = tonumber(ARGV[arg_index])
  arg_index = arg_index + 1
  local tpm = tonumber(ARGV[arg_index])
  arg_index = arg_index + 1

  local key_offset = (scope_index - 1) * 4
  local req_cur = tonumber(redis.call("GET", KEYS[key_offset + 1]) or "0")
  local req_prev = tonumber(redis.call("GET", KEYS[key_offset + 2]) or "0")
  local tok_cur = tonumber(redis.call("GET", KEYS[key_offset + 3]) or "0")
  local tok_prev = tonumber(redis.call("GET", KEYS[key_offset + 4]) or "0")

  local next_req_cur = req_cur + 1
  local next_tok_cur = tok_cur + tokens
  local weighted_req = next_req_cur * 60 + req_prev * window_weight
  local weighted_tok = next_tok_cur * 60 + tok_prev * window_weight

  if rpm == 0 then
    return {2, scope_index}
  end
  if rpm ~= nil and rpm > 0 and weighted_req > (rpm * 60) then
    return {2, scope_index}
  end

  if tpm == 0 then
    return {3, scope_index}
  end
  if tpm ~= nil and tpm > 0 and weighted_tok > (tpm * 60) then
    return {3, scope_index}
  end

  updates[scope_index] = {next_req_cur, next_tok_cur}
end

for scope_index = 1, scope_count do
  local key_offset = (scope_index - 1) * 4
  redis.call("SET", KEYS[key_offset + 1], updates[scope_index][1], "EX", ttl)
  redis.call("SET", KEYS[key_offset + 3], updates[scope_index][2], "EX", ttl)
end

return {1, 0}
"#;

#[cfg(feature = "gateway-store-redis")]
fn limit_to_i64(value: Option<u32>) -> i64 {
    value.map(i64::from).unwrap_or(-1)
}

#[cfg(feature = "gateway-store-redis")]
fn tokens_u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

#[cfg(feature = "gateway-store-redis")]
impl RedisStore {
    fn key_rate_limit_requests(&self, key_id: &str, route: &str, minute: u64) -> String {
        format!("{}:rate_limit:req:{key_id}:{route}:{minute}", self.prefix)
    }

    fn key_rate_limit_tokens(&self, key_id: &str, route: &str, minute: u64) -> String {
        format!("{}:rate_limit:tok:{key_id}:{route}:{minute}", self.prefix)
    }

    pub async fn check_and_consume_rate_limits(
        &self,
        key_id: &str,
        route: &str,
        limits: &super::LimitsConfig,
        tokens: u32,
        now_epoch_seconds: u64,
    ) -> Result<(), super::GatewayError> {
        if limits.rpm.is_none() && limits.tpm.is_none() {
            return Ok(());
        }

        let minute = now_epoch_seconds / 60;
        let prev_minute = minute.saturating_sub(1);
        let second_in_minute = (now_epoch_seconds % 60).min(59);

        let req_cur_key = self.key_rate_limit_requests(key_id, route, minute);
        let req_prev_key = self.key_rate_limit_requests(key_id, route, prev_minute);
        let tok_cur_key = self.key_rate_limit_tokens(key_id, route, minute);
        let tok_prev_key = self.key_rate_limit_tokens(key_id, route, prev_minute);

        let mut conn = self
            .connection()
            .await
            .map_err(|err| super::GatewayError::Backend {
                message: format!("redis error: {err}"),
            })?;

        let rpm = limit_to_i64(limits.rpm);
        let tpm = limit_to_i64(limits.tpm);
        let tokens = tokens_u32_to_i64(tokens);
        let second = i64::from(second_in_minute as u32);

        let script = redis::Script::new(RATE_LIMIT_SCRIPT);
        let code: i64 = script
            .key(req_cur_key)
            .key(req_prev_key)
            .key(tok_cur_key)
            .key(tok_prev_key)
            .arg(rpm)
            .arg(tpm)
            .arg(tokens)
            .arg(second)
            .arg(DEFAULT_RATE_LIMIT_TTL_SECS as i64)
            .invoke_async(&mut conn)
            .await
            .map_err(|err| super::GatewayError::Backend {
                message: format!("redis error: {err}"),
            })?;

        match code {
            1 => Ok(()),
            2 => Err(super::GatewayError::RateLimited {
                limit: format!("rpm>{}", limits.rpm.unwrap_or(0)),
            }),
            3 => Err(super::GatewayError::RateLimited {
                limit: format!("tpm>{}", limits.tpm.unwrap_or(0)),
            }),
            _ => Err(super::GatewayError::Backend {
                message: format!("unexpected rate limit script response: {code}"),
            }),
        }
    }

    pub async fn check_and_consume_rate_limits_many<'a, I>(
        &self,
        scopes: I,
        route: &str,
        tokens: u32,
        now_epoch_seconds: u64,
    ) -> Result<(), super::GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a super::LimitsConfig)>,
    {
        let scoped_limits = scopes
            .into_iter()
            .filter(|(_, limits)| limits.rpm.is_some() || limits.tpm.is_some())
            .collect::<Vec<_>>();
        if scoped_limits.is_empty() {
            return Ok(());
        }

        let minute = now_epoch_seconds / 60;
        let prev_minute = minute.saturating_sub(1);
        let second_in_minute = (now_epoch_seconds % 60).min(59);

        let mut conn = self
            .connection()
            .await
            .map_err(|err| super::GatewayError::Backend {
                message: format!("redis error: {err}"),
            })?;

        let script = redis::Script::new(RATE_LIMIT_MANY_SCRIPT);
        let mut invocation = script.prepare_invoke();
        invocation.arg(i64::try_from(scoped_limits.len()).unwrap_or(i64::MAX));
        invocation.arg(tokens_u32_to_i64(tokens));
        invocation.arg(i64::from(second_in_minute as u32));
        invocation.arg(DEFAULT_RATE_LIMIT_TTL_SECS as i64);

        for (scope, limits) in &scoped_limits {
            invocation.key(self.key_rate_limit_requests(scope, route, minute));
            invocation.key(self.key_rate_limit_requests(scope, route, prev_minute));
            invocation.key(self.key_rate_limit_tokens(scope, route, minute));
            invocation.key(self.key_rate_limit_tokens(scope, route, prev_minute));
            invocation.arg(limit_to_i64(limits.rpm));
            invocation.arg(limit_to_i64(limits.tpm));
        }

        let result: Vec<i64> = invocation.invoke_async(&mut conn).await.map_err(|err| {
            super::GatewayError::Backend {
                message: format!("redis error: {err}"),
            }
        })?;

        let code = result.first().copied().unwrap_or_default();
        let scope_index: Option<usize> = result
            .get(1)
            .copied()
            .and_then(|index| usize::try_from(index).ok());

        match code {
            1 => Ok(()),
            2 => {
                let Some(scope_offset) = scope_index.and_then(|index| index.checked_sub(1)) else {
                    return Err(super::GatewayError::Backend {
                        message: format!("unexpected batched rate limit response: {:?}", result),
                    });
                };
                let Some((_, limits)) = scoped_limits.get(scope_offset) else {
                    return Err(super::GatewayError::Backend {
                        message: format!("unexpected batched rate limit response: {:?}", result),
                    });
                };
                Err(super::GatewayError::RateLimited {
                    limit: format!("rpm>{}", limits.rpm.unwrap_or(0)),
                })
            }
            3 => {
                let Some(scope_offset) = scope_index.and_then(|index| index.checked_sub(1)) else {
                    return Err(super::GatewayError::Backend {
                        message: format!("unexpected batched rate limit response: {:?}", result),
                    });
                };
                let Some((_, limits)) = scoped_limits.get(scope_offset) else {
                    return Err(super::GatewayError::Backend {
                        message: format!("unexpected batched rate limit response: {:?}", result),
                    });
                };
                Err(super::GatewayError::RateLimited {
                    limit: format!("tpm>{}", limits.tpm.unwrap_or(0)),
                })
            }
            _ => Err(super::GatewayError::Backend {
                message: format!("unexpected batched rate limit response: {:?}", result),
            }),
        }
    }
}
// end inline: ../../../redis_store/rate_limits.rs
// inlined from ../../../redis_store/audit.rs
impl RedisStore {
    pub async fn append_audit_log(
        &self,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let kind = kind.into();
        let ts_ms_i64 = now_millis();
        let ts_ms = if ts_ms_i64 <= 0 { 0 } else { ts_ms_i64 as u64 };
        let id: i64 = conn.incr(self.key_audit_seq(), 1).await?;
        let member = format!("{id:020}");
        let record = AuditLogRecord {
            id,
            ts_ms,
            kind,
            payload,
        };
        let serialized = serde_json::to_string(&record)?;

        let record_key = self.key_audit_record(&member);
        let idx_key = self.key_audit_by_ts();

        let retention_secs = self.audit_retention_secs;
        let should_reap = should_run_retention_reap(&self.audit_last_retention_reap_ms, ts_ms_i64);
        let mut pipe = redis::pipe();
        pipe.atomic();
        if let Some(retention_secs) = retention_secs {
            pipe.cmd("SET")
                .arg(&record_key)
                .arg(&serialized)
                .arg("EX")
                .arg(retention_secs.max(1));
        } else {
            pipe.set(&record_key, &serialized);
        }
        pipe.zadd(&idx_key, member, ts_ms);
        if should_reap && let Some(cutoff_ms) = audit_cutoff_ms(retention_secs, ts_ms) {
            pipe.cmd("ZREMRANGEBYSCORE")
                .arg(&idx_key)
                .arg("-inf")
                .arg(cutoff_ms);
        }
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn list_audit_logs(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let idx_key = self.key_audit_by_ts();
        let limit = limit.clamp(1, 1000);

        let members: Vec<String> = if let Some(since) = since_ts_ms {
            redis::cmd("ZREVRANGEBYSCORE")
                .arg(&idx_key)
                .arg("+inf")
                .arg(since)
                .arg("LIMIT")
                .arg(0)
                .arg(limit)
                .query_async(&mut conn)
                .await?
        } else {
            redis::cmd("ZREVRANGEBYSCORE")
                .arg(&idx_key)
                .arg("+inf")
                .arg("-inf")
                .arg("LIMIT")
                .arg(0)
                .arg(limit)
                .query_async(&mut conn)
                .await?
        };

        let mut out = Vec::with_capacity(members.len());
        for member in members {
            let record_key = self.key_audit_record(&member);
            let raw: Option<String> = conn.get(record_key).await?;
            let Some(raw) = raw else {
                continue;
            };
            out.push(serde_json::from_str(&raw)?);
        }
        Ok(out)
    }

    pub async fn list_audit_logs_window(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
        before_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let idx_key = self.key_audit_by_ts();
        let limit = limit.clamp(1, 10_000);

        let max = before_ts_ms
            .map(|value| format!("({value}"))
            .unwrap_or_else(|| "+inf".to_string());
        let min = since_ts_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-inf".to_string());

        let members: Vec<String> = redis::cmd("ZREVRANGEBYSCORE")
            .arg(&idx_key)
            .arg(max)
            .arg(min)
            .arg("LIMIT")
            .arg(0)
            .arg(limit)
            .query_async(&mut conn)
            .await?;

        let mut out = Vec::with_capacity(members.len());
        for member in members {
            let record_key = self.key_audit_record(&member);
            let raw: Option<String> = conn.get(record_key).await?;
            let Some(raw) = raw else {
                continue;
            };
            out.push(serde_json::from_str(&raw)?);
        }
        Ok(out)
    }

    pub async fn list_cost_ledgers(&self) -> Result<Vec<CostLedgerRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let mut key_ids: Vec<String> = conn.smembers(self.key_cost_keys()).await?;
        key_ids.sort();

        let mut out = Vec::with_capacity(key_ids.len());
        for key_id in key_ids {
            let ledger_key = self.key_cost_ledger(&key_id);
            let raw: HashMap<String, String> = conn.hgetall(ledger_key).await?;
            let spent_usd_micros = raw
                .get("spent_usd_micros")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let reserved_usd_micros = raw
                .get("reserved_usd_micros")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let updated_at_ms = raw
                .get("updated_at_ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            out.push(CostLedgerRecord {
                key_id,
                spent_usd_micros,
                reserved_usd_micros,
                updated_at_ms,
            });
        }
        Ok(out)
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
fn now_millis_u64() -> u64 {
    let now_ms = now_millis();
    if now_ms <= 0 { 0 } else { now_ms as u64 }
}

fn tokens_to_i64(tokens: u64) -> i64 {
    if tokens > i64::MAX as u64 {
        i64::MAX
    } else {
        tokens as i64
    }
}
// end inline: ../../../redis_store/audit.rs
// inlined from ../../../redis_store/tests.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "gateway-proxy-cache")]
    #[test]
    fn proxy_cache_record_round_trips_headers_body_and_metadata() {
        let mut headers = axum::http::HeaderMap::new();
        headers.append("content-type", "application/json".parse().unwrap());
        headers.append("set-cookie", "a=b".parse().unwrap());

        let cached = CachedProxyResponse {
            status: 200,
            headers: headers.clone(),
            body: bytes::Bytes::from_static(b"ok"),
            backend: "primary".to_string(),
        };
        let metadata = ProxyCacheEntryMetadata::new(
            "vk:key-1",
            &axum::http::Method::POST,
            "/v1/responses?stream=false",
            Some("gpt-4o-mini"),
        );

        let record = CachedProxyResponseRecord::from_cached(&cached, &metadata);
        let raw = serde_json::to_vec(&record).expect("serialize");
        let decoded: CachedProxyResponseRecord = serde_json::from_slice(&raw).expect("decode");
        let round_tripped = decoded.into_stored();

        assert_eq!(round_tripped.response.status, cached.status);
        assert_eq!(round_tripped.response.backend, cached.backend);
        assert_eq!(round_tripped.response.body, cached.body);
        assert_eq!(round_tripped.metadata, metadata);

        assert_eq!(
            round_tripped.response.headers.get("content-type"),
            headers.get("content-type")
        );
        assert_eq!(
            round_tripped
                .response
                .headers
                .get_all("set-cookie")
                .iter()
                .collect::<Vec<_>>(),
            headers.get_all("set-cookie").iter().collect::<Vec<_>>(),
        );
    }

    fn env_nonempty(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn redis_url() -> Option<String> {
        env_nonempty("DITTO_REDIS_URL").or_else(|| env_nonempty("REDIS_URL"))
    }

    fn required_redis_url() -> Option<String> {
        redis_url()
    }

    static PREFIX_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_prefix() -> String {
        let n = PREFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("ditto_test:{}:{n}", now_millis())
    }

    #[tokio::test]
    async fn redis_store_round_trips_virtual_keys_and_budget_ledgers() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let key = VirtualKeyConfig::new("key-1", "vk-1");
        store
            .replace_virtual_keys(std::slice::from_ref(&key))
            .await
            .expect("persist");
        let loaded = store.load_virtual_keys().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "key-1");

        let router = super::super::RouterConfig {
            default_backends: Vec::new(),
            rules: Vec::new(),
        };
        store
            .replace_router_config(&router)
            .await
            .expect("persist router");
        let loaded_router = store.load_router_config().await.expect("load router");
        assert!(loaded_router.is_some());
        let loaded_router = loaded_router.expect("router");
        assert_eq!(loaded_router.default_backends.len(), 0);
        assert_eq!(loaded_router.rules.len(), 0);

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 5)
            .await
            .expect("reserve");
        store
            .commit_budget_reservation("req-1")
            .await
            .expect("commit");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_tokens, 5);
        assert_eq!(ledgers[0].reserved_tokens, 0);
    }

    #[tokio::test]
    async fn redis_store_commit_budget_reservation_with_tokens_releases_difference() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");
        store
            .commit_budget_reservation_with_tokens("req-1", 3)
            .await
            .expect("commit");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_tokens, 3);
        assert_eq!(ledgers[0].reserved_tokens, 0);

        store
            .reserve_budget_tokens("req-4", "key-1", 20, 2)
            .await
            .expect("reserve overspend");
        store
            .commit_budget_reservation_with_tokens("req-4", 5)
            .await
            .expect("commit overspend");

        let ledgers = store.list_budget_ledgers().await.expect("updated ledgers");
        assert_eq!(ledgers[0].spent_tokens, 8);
        assert_eq!(ledgers[0].reserved_tokens, 0);

        store
            .reserve_budget_tokens("req-2", "key-1", 10, 7)
            .await
            .expect("reserve 2");
        let err = store.reserve_budget_tokens("req-3", "key-1", 10, 1).await;
        assert!(matches!(err, Err(RedisStoreError::BudgetExceeded { .. })));
    }

    #[tokio::test]
    async fn redis_store_commit_cost_reservation_with_usd_micros_releases_difference() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");
        store
            .commit_cost_reservation_with_usd_micros("req-1", 3)
            .await
            .expect("commit");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].spent_usd_micros, 3);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);

        store
            .reserve_cost_usd_micros("req-4", "key-1", 20, 2)
            .await
            .expect("reserve overspend");
        store
            .commit_cost_reservation_with_usd_micros("req-4", 5)
            .await
            .expect("commit overspend");

        let ledgers = store.list_cost_ledgers().await.expect("updated ledgers");
        assert_eq!(ledgers[0].spent_usd_micros, 8);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);

        store
            .reserve_cost_usd_micros("req-2", "key-1", 10, 7)
            .await
            .expect("reserve 2");
        let err = store.reserve_cost_usd_micros("req-3", "key-1", 10, 1).await;
        assert!(matches!(
            err,
            Err(RedisStoreError::CostBudgetExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn redis_store_reaps_stale_budget_reservations() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_budget_tokens("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].reserved_tokens, 7);

        let cutoff_ts_ms = now_millis_u64().saturating_add(1);
        let (_scanned, reaped, released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, 1000, false)
            .await
            .expect("reap");
        assert_eq!(reaped, 1);
        assert_eq!(released, 7);

        let ledgers = store.list_budget_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].reserved_tokens, 0);
        assert_eq!(ledgers[0].spent_tokens, 0);
    }

    #[tokio::test]
    async fn redis_store_reaps_stale_cost_reservations() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        store
            .reserve_cost_usd_micros("req-1", "key-1", 10, 7)
            .await
            .expect("reserve");

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].key_id, "key-1");
        assert_eq!(ledgers[0].reserved_usd_micros, 7);

        let cutoff_ts_ms = now_millis_u64().saturating_add(1);
        let (_scanned, reaped, released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, 1000, false)
            .await
            .expect("reap");
        assert_eq!(reaped, 1);
        assert_eq!(released, 7);

        let ledgers = store.list_cost_ledgers().await.expect("ledgers");
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0].reserved_usd_micros, 0);
        assert_eq!(ledgers[0].spent_usd_micros, 0);
    }

    #[tokio::test]
    async fn redis_store_rate_limits_enforce_rpm() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let now_epoch_seconds = now_millis_u64() / 1000;
        let limits = super::super::LimitsConfig {
            rpm: Some(2),
            tpm: Some(1000),
        };

        store
            .check_and_consume_rate_limits(
                "key-rpm",
                "/v1/chat/completions",
                &limits,
                1,
                now_epoch_seconds,
            )
            .await
            .expect("first allowed");
        store
            .check_and_consume_rate_limits(
                "key-rpm",
                "/v1/chat/completions",
                &limits,
                1,
                now_epoch_seconds,
            )
            .await
            .expect("second allowed");

        let err = store
            .check_and_consume_rate_limits(
                "key-rpm",
                "/v1/chat/completions",
                &limits,
                1,
                now_epoch_seconds,
            )
            .await
            .expect_err("third blocked");
        assert!(matches!(
            err,
            super::super::GatewayError::RateLimited { .. }
        ));
        if let super::super::GatewayError::RateLimited { limit } = err {
            assert!(limit.starts_with("rpm>"));
        }
    }

    #[tokio::test]
    async fn redis_store_rate_limits_enforce_tpm() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let now_epoch_seconds = now_millis_u64() / 1000;
        let limits = super::super::LimitsConfig {
            rpm: Some(1000),
            tpm: Some(3),
        };

        store
            .check_and_consume_rate_limits(
                "key-tpm",
                "/v1/embeddings",
                &limits,
                2,
                now_epoch_seconds,
            )
            .await
            .expect("first allowed");

        let err = store
            .check_and_consume_rate_limits(
                "key-tpm",
                "/v1/embeddings",
                &limits,
                2,
                now_epoch_seconds,
            )
            .await
            .expect_err("second blocked");
        assert!(matches!(
            err,
            super::super::GatewayError::RateLimited { .. }
        ));
        if let super::super::GatewayError::RateLimited { limit } = err {
            assert!(limit.starts_with("tpm>"));
        }
    }

    #[tokio::test]
    async fn redis_store_rate_limits_many_is_atomic_across_scopes() {
        let Some(url) = required_redis_url() else {
            eprintln!("skipping redis test: set DITTO_REDIS_URL or REDIS_URL");
            return;
        };

        let store = RedisStore::new(url)
            .expect("store")
            .with_prefix(test_prefix());
        store.ping().await.expect("ping");

        let now_epoch_seconds = now_millis_u64() / 1000;
        let route = "/v1/chat/completions";
        let shared_limits = super::super::LimitsConfig {
            rpm: Some(2),
            tpm: Some(1000),
        };
        let tight_limits = super::super::LimitsConfig {
            rpm: Some(1),
            tpm: Some(1000),
        };

        store
            .check_and_consume_rate_limits_many(
                [("key-many", &shared_limits), ("tenant:t1", &shared_limits)],
                route,
                1,
                now_epoch_seconds,
            )
            .await
            .expect("first batched request allowed");

        store
            .check_and_consume_rate_limits("user:u1", route, &tight_limits, 1, now_epoch_seconds)
            .await
            .expect("tight scope primed");

        let err = store
            .check_and_consume_rate_limits_many(
                [("key-many", &shared_limits), ("user:u1", &tight_limits)],
                route,
                1,
                now_epoch_seconds,
            )
            .await
            .expect_err("batched request should be rejected atomically");
        assert!(matches!(
            err,
            super::super::GatewayError::RateLimited { .. }
        ));
        if let super::super::GatewayError::RateLimited { limit } = err {
            assert!(limit.starts_with("rpm>"));
        }

        store
            .check_and_consume_rate_limits("key-many", route, &shared_limits, 1, now_epoch_seconds)
            .await
            .expect("failed batched request must not consume key scope");
    }
}
// end inline: ../../../redis_store/tests.rs
