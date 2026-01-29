use std::collections::HashMap;

use redis::AsyncCommands;
use thiserror::Error;

use super::{AuditLogRecord, BudgetLedgerRecord, CostLedgerRecord, VirtualKeyConfig};

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
            pipe.hset(&redis_key, &key.id, serde_json::to_string(key)?);
        }
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

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
                    .unwrap_or(limit.saturating_add(tokens));
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
                    .unwrap_or(limit_usd_micros.saturating_add(usd_micros));
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
redis.call("HINCRBY", ledger_key, "spent_tokens", tokens)
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

    pub async fn commit_cost_reservation(&self, request_id: &str) -> Result<(), RedisStoreError> {
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
redis.call("HINCRBY", ledger_key, "spent_usd_micros", usd_micros)
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

    pub async fn append_audit_log(
        &self,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let kind = kind.into();
        let ts_ms = now_millis_u64();
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

        let _: () = redis::pipe()
            .atomic()
            .set(&record_key, serialized)
            .zadd(&idx_key, member, ts_ms)
            .query_async(&mut conn)
            .await?;
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

fn now_millis_u64() -> u64 {
    if now_millis() <= 0 {
        0
    } else {
        now_millis() as u64
    }
}

fn tokens_to_i64(tokens: u64) -> i64 {
    if tokens > i64::MAX as u64 {
        i64::MAX
    } else {
        tokens as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_nonempty(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    #[tokio::test]
    async fn redis_store_round_trips_virtual_keys_and_budget_ledgers() {
        let Some(url) = env_nonempty("DITTO_REDIS_URL").or_else(|| env_nonempty("REDIS_URL"))
        else {
            return;
        };

        let prefix = format!("ditto_test:{}", now_millis());
        let store = RedisStore::new(url).expect("store").with_prefix(prefix);
        store.ping().await.expect("ping");

        let key = VirtualKeyConfig::new("key-1", "vk-1");
        store
            .replace_virtual_keys(std::slice::from_ref(&key))
            .await
            .expect("persist");
        let loaded = store.load_virtual_keys().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "key-1");

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
}
