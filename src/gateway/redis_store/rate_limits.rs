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
}
