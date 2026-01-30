use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyRetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_retry_status_codes")]
    pub retry_status_codes: Vec<u16>,
    #[serde(default)]
    pub max_attempts: Option<usize>,
}

fn default_retry_status_codes() -> Vec<u16> {
    vec![429, 500, 502, 503, 504]
}

impl Default for ProxyRetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retry_status_codes: default_retry_status_codes(),
            max_attempts: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyCircuitBreakerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_cooldown_seconds() -> u64 {
    30
}

impl Default for ProxyCircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_threshold: default_failure_threshold(),
            cooldown_seconds: default_cooldown_seconds(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyHealthCheckConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_health_check_path")]
    pub path: String,
    #[serde(default = "default_health_check_interval_seconds")]
    pub interval_seconds: u64,
    #[serde(default = "default_health_check_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_health_check_path() -> String {
    "/v1/models".to_string()
}

fn default_health_check_interval_seconds() -> u64 {
    10
}

fn default_health_check_timeout_seconds() -> u64 {
    2
}

impl Default for ProxyHealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_health_check_path(),
            interval_seconds: default_health_check_interval_seconds(),
            timeout_seconds: default_health_check_timeout_seconds(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxyRoutingConfig {
    #[serde(default)]
    pub retry: ProxyRetryConfig,
    #[serde(default)]
    pub circuit_breaker: ProxyCircuitBreakerConfig,
    #[serde(default)]
    pub health_check: ProxyHealthCheckConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendHealthSnapshot {
    pub backend: String,
    pub consecutive_failures: u32,
    pub unhealthy_until_epoch_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_ts_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_healthy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_last_ts_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct BackendHealth {
    consecutive_failures: u32,
    unhealthy_until_epoch_seconds: Option<u64>,
    last_error: Option<String>,
    last_failure_ts_ms: Option<u64>,
    health_check_healthy: Option<bool>,
    health_check_last_error: Option<String>,
    health_check_last_ts_ms: Option<u64>,
}

impl BackendHealth {
    pub fn snapshot(&self, backend: &str) -> BackendHealthSnapshot {
        BackendHealthSnapshot {
            backend: backend.to_string(),
            consecutive_failures: self.consecutive_failures,
            unhealthy_until_epoch_seconds: self.unhealthy_until_epoch_seconds,
            last_error: self.last_error.clone(),
            last_failure_ts_ms: self.last_failure_ts_ms,
            health_check_healthy: self.health_check_healthy,
            health_check_last_error: self.health_check_last_error.clone(),
            health_check_last_ts_ms: self.health_check_last_ts_ms,
        }
    }

    pub fn is_healthy(&self, now_epoch_seconds: u64) -> bool {
        if self.health_check_healthy == Some(false) {
            return false;
        }
        match self.unhealthy_until_epoch_seconds {
            Some(until) => now_epoch_seconds >= until,
            None => true,
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.unhealthy_until_epoch_seconds = None;
        self.last_error = None;
        self.last_failure_ts_ms = None;
    }

    pub fn record_failure(
        &mut self,
        now_epoch_seconds: u64,
        circuit_breaker: &ProxyCircuitBreakerConfig,
        kind: FailureKind,
        message: String,
    ) {
        self.last_error = Some(message);
        self.last_failure_ts_ms = Some(now_millis());

        if !circuit_breaker.enabled {
            return;
        }

        let should_count = match kind {
            FailureKind::Network => true,
            FailureKind::RetryableStatus(code) => code >= 500,
        };
        if !should_count {
            return;
        }

        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= circuit_breaker.failure_threshold {
            self.unhealthy_until_epoch_seconds =
                Some(now_epoch_seconds.saturating_add(circuit_breaker.cooldown_seconds));
        }
    }

    pub fn record_health_check_success(&mut self) {
        self.health_check_healthy = Some(true);
        self.health_check_last_error = None;
        self.health_check_last_ts_ms = Some(now_millis());
    }

    pub fn record_health_check_failure(&mut self, message: String) {
        self.health_check_healthy = Some(false);
        self.health_check_last_error = Some(message);
        self.health_check_last_ts_ms = Some(now_millis());
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FailureKind {
    Network,
    RetryableStatus(u16),
}

pub fn filter_healthy_backends(
    candidates: &[String],
    health: &HashMap<String, BackendHealth>,
    now_epoch_seconds: u64,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|name| {
            health
                .get(name.as_str())
                .map(|state| state.is_healthy(now_epoch_seconds))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
