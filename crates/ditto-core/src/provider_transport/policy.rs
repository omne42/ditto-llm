#![cfg_attr(
    not(any(
        feature = "anthropic",
        feature = "cohere",
        feature = "google",
        feature = "openai",
        feature = "openai-compatible",
        feature = "bedrock",
        feature = "vertex"
    )),
    allow(dead_code)
)]

use std::time::Duration;

use serde::{Deserialize, Serialize};

// PROVIDER-TRANSPORT-POLICY-OWNER: provider-facing HTTP client/body limits live
// here as machine-readable transport policy, instead of staying as scattered
// magic constants inside helper functions.

pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpClientPolicy {
    pub timeout_ms: u64,
}

impl HttpClientPolicy {
    pub const fn new(timeout_ms: u64) -> Self {
        Self { timeout_ms }
    }

    pub const fn default_timeout_ms() -> u64 {
        DEFAULT_HTTP_TIMEOUT_SECS * 1000
    }

    pub fn timeout(self) -> Duration {
        Duration::from_millis(self.timeout_ms.max(1))
    }

    #[cfg_attr(
        not(any(
            feature = "anthropic",
            feature = "bedrock",
            feature = "cohere",
            feature = "google",
            feature = "openai",
            feature = "openai-compatible",
            feature = "vertex"
        )),
        allow(dead_code)
    )]
    pub(crate) fn from_timeout(timeout: Duration) -> Self {
        let timeout_ms = timeout.as_millis().try_into().unwrap_or(u64::MAX);
        Self::new(timeout_ms)
    }
}

impl Default for HttpClientPolicy {
    fn default() -> Self {
        Self::new(Self::default_timeout_ms())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponseBodyPolicy {
    pub max_error_body_bytes: usize,
    pub max_response_body_bytes: usize,
}

impl HttpResponseBodyPolicy {
    pub const fn new(max_error_body_bytes: usize, max_response_body_bytes: usize) -> Self {
        Self {
            max_error_body_bytes,
            max_response_body_bytes,
        }
    }
}

impl Default for HttpResponseBodyPolicy {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_ERROR_BODY_BYTES,
            DEFAULT_MAX_RESPONSE_BODY_BYTES,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HttpTransportPolicy {
    pub client: HttpClientPolicy,
    pub body: HttpResponseBodyPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_http_transport_policy_matches_current_runtime_defaults() {
        let policy = HttpTransportPolicy::default();
        assert_eq!(policy.client.timeout_ms, 300_000);
        assert_eq!(
            policy.body.max_error_body_bytes,
            DEFAULT_MAX_ERROR_BODY_BYTES
        );
        assert_eq!(
            policy.body.max_response_body_bytes,
            DEFAULT_MAX_RESPONSE_BODY_BYTES
        );
    }
}
