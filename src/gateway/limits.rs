use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::GatewayError;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LimitsConfig {
    pub rpm: Option<u32>,
    pub tpm: Option<u32>,
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    usage: HashMap<String, MinuteUsage>,
    last_gc_minute: u64,
}

#[derive(Debug, Clone)]
struct MinuteUsage {
    minute: u64,
    requests: u32,
    tokens: u32,
}

impl RateLimiter {
    pub fn check_and_consume(
        &mut self,
        key_id: &str,
        limits: &LimitsConfig,
        tokens: u32,
        minute: u64,
    ) -> Result<(), GatewayError> {
        if limits.rpm.is_none() && limits.tpm.is_none() {
            // If limits are disabled for this scope, stop retaining per-minute state for it.
            self.usage.remove(key_id);
            return Ok(());
        }

        if minute != self.last_gc_minute {
            // Keep only the active minute bucket. Older/future buckets are stale.
            self.usage.retain(|_, usage| usage.minute == minute);
            self.last_gc_minute = minute;
        }

        let usage = self.usage.entry(key_id.to_string()).or_insert(MinuteUsage {
            minute,
            requests: 0,
            tokens: 0,
        });

        if usage.minute != minute {
            usage.minute = minute;
            usage.requests = 0;
            usage.tokens = 0;
        }

        let next_requests = usage.requests.saturating_add(1);
        let next_tokens = usage.tokens.saturating_add(tokens);

        if let Some(rpm) = limits.rpm {
            if rpm == 0 || next_requests > rpm {
                return Err(GatewayError::RateLimited {
                    limit: format!("rpm>{rpm}"),
                });
            }
        }

        if let Some(tpm) = limits.tpm {
            if tpm == 0 || next_tokens > tpm {
                return Err(GatewayError::RateLimited {
                    limit: format!("tpm>{tpm}"),
                });
            }
        }

        usage.requests = next_requests;
        usage.tokens = next_tokens;
        Ok(())
    }

    pub fn retain_scopes(&mut self, scopes: &HashSet<String>) {
        self.usage.retain(|scope, _| scopes.contains(scope));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_keeps_only_current_minute_after_clock_rollback() {
        let mut limiter = RateLimiter::default();
        let limits = LimitsConfig {
            rpm: Some(10),
            tpm: Some(100),
        };

        limiter.check_and_consume("a", &limits, 1, 100).unwrap();
        limiter.check_and_consume("b", &limits, 1, 99).unwrap();

        assert_eq!(limiter.usage.len(), 1);
        assert_eq!(limiter.usage.get("b").map(|usage| usage.minute), Some(99));
    }

    #[test]
    fn no_limits_call_drops_stale_usage_for_scope() {
        let mut limiter = RateLimiter::default();
        let limited = LimitsConfig {
            rpm: Some(10),
            tpm: None,
        };

        limiter.check_and_consume("scope", &limited, 1, 42).unwrap();
        assert!(limiter.usage.contains_key("scope"));

        limiter
            .check_and_consume("scope", &LimitsConfig::default(), 1, 42)
            .unwrap();
        assert!(!limiter.usage.contains_key("scope"));
    }
}
