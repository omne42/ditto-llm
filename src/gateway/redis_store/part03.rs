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

local committed_tokens = reserved_tokens
if spent_tokens < committed_tokens then
  committed_tokens = spent_tokens
end

local ledger_key = prefix .. ":budget_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_tokens", -reserved_tokens) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_tokens", 0)
end
redis.call("HINCRBY", ledger_key, "spent_tokens", committed_tokens)
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

local committed_usd_micros = reserved_usd_micros
if spent_usd_micros < committed_usd_micros then
  committed_usd_micros = spent_usd_micros
end

local ledger_key = prefix .. ":cost_ledger:" .. key_id
local reserved_after = tonumber(redis.call("HINCRBY", ledger_key, "reserved_usd_micros", -reserved_usd_micros) or "0") or 0
if reserved_after < 0 then
  redis.call("HSET", ledger_key, "reserved_usd_micros", 0)
end
redis.call("HINCRBY", ledger_key, "spent_usd_micros", committed_usd_micros)
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
}
