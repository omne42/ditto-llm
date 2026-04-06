use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::GatewayError;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LimitsConfig {
    pub rpm: Option<u32>,
    pub tpm: Option<u32>,
}

#[derive(Clone, Debug, Default)]
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
    fn gc_if_needed(&mut self, minute: u64) {
        if minute > self.last_gc_minute {
            // Only advance GC when time moves forward so out-of-order or rolled-back
            // requests cannot drop newer buckets for other scopes.
            self.usage.retain(|_, usage| usage.minute == minute);
            self.last_gc_minute = minute;
        }
    }

    fn usage_for_scope(&self, scope: &str, minute: u64) -> MinuteUsage {
        match self.usage.get(scope) {
            Some(usage) if usage.minute == minute => usage.clone(),
            _ => MinuteUsage {
                minute,
                requests: 0,
                tokens: 0,
            },
        }
    }

    fn validate_next_usage(
        limits: &LimitsConfig,
        current: &MinuteUsage,
        tokens: u32,
    ) -> Result<MinuteUsage, GatewayError> {
        let next_requests = current.requests.saturating_add(1);
        let next_tokens = current.tokens.saturating_add(tokens);

        if let Some(rpm) = limits.rpm
            && (rpm == 0 || next_requests > rpm)
        {
            return Err(GatewayError::RateLimited {
                limit: format!("rpm>{rpm}"),
            });
        }

        if let Some(tpm) = limits.tpm
            && (tpm == 0 || next_tokens > tpm)
        {
            return Err(GatewayError::RateLimited {
                limit: format!("tpm>{tpm}"),
            });
        }

        Ok(MinuteUsage {
            minute: current.minute,
            requests: next_requests,
            tokens: next_tokens,
        })
    }

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

        self.gc_if_needed(minute);
        let usage = self.usage_for_scope(key_id, minute);
        let next_usage = Self::validate_next_usage(limits, &usage, tokens)?;
        self.usage.insert(key_id.to_string(), next_usage);
        Ok(())
    }

    pub fn check_and_consume_many<'a, I>(
        &mut self,
        scopes: I,
        tokens: u32,
        minute: u64,
    ) -> Result<(), GatewayError>
    where
        I: IntoIterator<Item = (&'a str, &'a LimitsConfig)>,
    {
        self.gc_if_needed(minute);

        let mut proposed = HashMap::<String, MinuteUsage>::new();
        let mut remove_scopes = Vec::<String>::new();

        for (scope, limits) in scopes {
            if limits.rpm.is_none() && limits.tpm.is_none() {
                remove_scopes.push(scope.to_string());
                continue;
            }

            let current = proposed
                .get(scope)
                .cloned()
                .unwrap_or_else(|| self.usage_for_scope(scope, minute));
            let next = Self::validate_next_usage(limits, &current, tokens)?;
            proposed.insert(scope.to_string(), next);
        }

        for scope in remove_scopes {
            self.usage.remove(&scope);
        }
        for (scope, usage) in proposed {
            self.usage.insert(scope, usage);
        }
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
    fn clock_rollback_does_not_drop_newer_buckets() {
        let mut limiter = RateLimiter::default();
        let limits = LimitsConfig {
            rpm: Some(10),
            tpm: Some(100),
        };

        limiter.check_and_consume("a", &limits, 1, 100).unwrap();
        limiter.check_and_consume("b", &limits, 1, 101).unwrap();
        limiter.check_and_consume("c", &limits, 1, 99).unwrap();

        assert_eq!(limiter.usage.len(), 2);
        assert_eq!(limiter.usage.get("b").map(|usage| usage.minute), Some(101));
        assert_eq!(limiter.usage.get("c").map(|usage| usage.minute), Some(99));
    }

    #[test]
    fn forward_gc_still_drops_older_buckets() {
        let mut limiter = RateLimiter::default();
        let limits = LimitsConfig {
            rpm: Some(10),
            tpm: Some(100),
        };

        limiter.check_and_consume("a", &limits, 1, 100).unwrap();
        limiter.check_and_consume("b", &limits, 1, 101).unwrap();

        assert_eq!(limiter.usage.len(), 1);
        assert_eq!(limiter.usage.get("b").map(|usage| usage.minute), Some(101));
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

    #[test]
    fn check_and_consume_many_is_atomic_across_scopes() {
        let mut limiter = RateLimiter::default();
        let limited = LimitsConfig {
            rpm: Some(10),
            tpm: Some(10),
        };
        let tight = LimitsConfig {
            rpm: Some(1),
            tpm: Some(10),
        };

        limiter
            .check_and_consume_many([("key", &limited), ("tenant:t1", &limited)], 5, 42)
            .unwrap();
        assert_eq!(
            limiter.usage.get("key").map(|usage| usage.requests),
            Some(1)
        );
        assert_eq!(
            limiter.usage.get("tenant:t1").map(|usage| usage.requests),
            Some(1)
        );

        limiter
            .check_and_consume("user:u1", &tight, 1, 42)
            .expect("prime tight scope");
        let err = limiter.check_and_consume_many([("key", &limited), ("user:u1", &tight)], 1, 42);
        assert!(matches!(err, Err(GatewayError::RateLimited { .. })));
        assert_eq!(
            limiter.usage.get("key").map(|usage| usage.requests),
            Some(1)
        );
        assert_eq!(
            limiter.usage.get("user:u1").map(|usage| usage.requests),
            Some(1)
        );
    }
}
