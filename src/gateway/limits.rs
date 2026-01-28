use std::collections::HashMap;

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
            return Ok(());
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
}
