use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyFailureAction {
    None,
    Fallback,
    Retry,
}

impl ProxyFailureAction {
    pub fn continues(self) -> bool {
        matches!(self, Self::Fallback | Self::Retry)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fallback => "fallback",
            Self::Retry => "retry",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Network,
    Timeout,
    Status(u16),
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Status(_) => "status",
        }
    }

    pub fn status_code(self) -> Option<u16> {
        match self {
            Self::Status(code) => Some(code),
            Self::Network | Self::Timeout => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProxyFailureDecision {
    pub action: ProxyFailureAction,
    pub kind: FailureKind,
}

impl ProxyFailureDecision {
    pub fn event_name(self) -> &'static str {
        match self.action {
            ProxyFailureAction::Retry => "proxy.retry",
            ProxyFailureAction::Fallback => "proxy.fallback",
            ProxyFailureAction::None => "proxy.route",
        }
    }

    pub fn reason_code(self) -> &'static str {
        match (self.kind, self.action) {
            (FailureKind::Network, _) => "network_error",
            (FailureKind::Timeout, _) => "timeout_error",
            (FailureKind::Status(_), ProxyFailureAction::Retry) => "retry_status",
            (FailureKind::Status(_), ProxyFailureAction::Fallback) => "fallback_status",
            (FailureKind::Status(_), ProxyFailureAction::None) => "status",
        }
    }

    pub fn should_attempt_next_backend(self, idx: usize, max_attempts: usize) -> bool {
        self.action.continues() && idx.saturating_add(1) < max_attempts
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyRetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_retry_status_codes")]
    pub retry_status_codes: Vec<u16>,
    #[serde(default)]
    pub fallback_status_codes: Vec<u16>,
    #[serde(default = "default_network_error_action")]
    pub network_error_action: ProxyFailureAction,
    #[serde(default = "default_timeout_error_action")]
    pub timeout_error_action: ProxyFailureAction,
    #[serde(default)]
    pub max_attempts: Option<usize>,
}

fn default_retry_status_codes() -> Vec<u16> {
    vec![429, 500, 502, 503, 504]
}

fn default_network_error_action() -> ProxyFailureAction {
    ProxyFailureAction::Fallback
}

fn default_timeout_error_action() -> ProxyFailureAction {
    ProxyFailureAction::Fallback
}

impl Default for ProxyRetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retry_status_codes: default_retry_status_codes(),
            fallback_status_codes: Vec::new(),
            network_error_action: default_network_error_action(),
            timeout_error_action: default_timeout_error_action(),
            max_attempts: None,
        }
    }
}

impl ProxyRetryConfig {
    pub fn action_for_status(&self, status: u16) -> ProxyFailureAction {
        if self.enabled && self.retry_status_codes.contains(&status) {
            return ProxyFailureAction::Retry;
        }
        if self.fallback_status_codes.contains(&status) {
            return ProxyFailureAction::Fallback;
        }
        ProxyFailureAction::None
    }

    pub fn action_for_failure(&self, kind: FailureKind) -> ProxyFailureAction {
        match kind {
            FailureKind::Network => self.network_error_action,
            FailureKind::Timeout => self.timeout_error_action,
            FailureKind::Status(status) => self.action_for_status(status),
        }
    }

    pub fn decision_for_failure(&self, kind: FailureKind) -> ProxyFailureDecision {
        ProxyFailureDecision {
            action: self.action_for_failure(kind),
            kind,
        }
    }

    pub fn should_retry_status(&self, status: u16) -> bool {
        self.action_for_status(status) == ProxyFailureAction::Retry
    }

    pub fn should_fallback_status(&self, status: u16) -> bool {
        self.action_for_status(status).continues()
    }

    pub fn is_failure_status(&self, status: u16) -> bool {
        self.action_for_status(status) != ProxyFailureAction::None
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
    #[serde(default = "default_count_network_errors")]
    pub count_network_errors: bool,
    #[serde(default = "default_count_timeout_errors")]
    pub count_timeout_errors: bool,
    #[serde(default = "default_count_server_errors")]
    pub count_server_errors: bool,
    #[serde(default)]
    pub failure_status_codes: Vec<u16>,
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_cooldown_seconds() -> u64 {
    30
}

fn default_count_network_errors() -> bool {
    true
}

fn default_count_timeout_errors() -> bool {
    true
}

fn default_count_server_errors() -> bool {
    true
}

impl Default for ProxyCircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_threshold: default_failure_threshold(),
            cooldown_seconds: default_cooldown_seconds(),
            count_network_errors: default_count_network_errors(),
            count_timeout_errors: default_count_timeout_errors(),
            count_server_errors: default_count_server_errors(),
            failure_status_codes: Vec::new(),
        }
    }
}

impl ProxyCircuitBreakerConfig {
    pub fn should_count_failure(&self, kind: FailureKind) -> bool {
        if !self.enabled {
            return false;
        }

        match kind {
            FailureKind::Network => self.count_network_errors,
            FailureKind::Timeout => self.count_timeout_errors,
            FailureKind::Status(code) => {
                (self.count_server_errors && code >= 500)
                    || self.failure_status_codes.contains(&code)
            }
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

        if !circuit_breaker.should_count_failure(kind) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_config_distinguishes_status_actions() {
        let config = ProxyRetryConfig {
            enabled: true,
            retry_status_codes: vec![429, 500],
            fallback_status_codes: vec![503],
            ..Default::default()
        };

        assert_eq!(config.action_for_status(429), ProxyFailureAction::Retry);
        assert_eq!(config.action_for_status(500), ProxyFailureAction::Retry);
        assert_eq!(config.action_for_status(503), ProxyFailureAction::Fallback);
        assert_eq!(config.action_for_status(404), ProxyFailureAction::None);
    }

    #[test]
    fn retry_config_uses_transport_actions() {
        let config = ProxyRetryConfig {
            network_error_action: ProxyFailureAction::None,
            timeout_error_action: ProxyFailureAction::Retry,
            ..Default::default()
        };

        assert_eq!(
            config.action_for_failure(FailureKind::Network),
            ProxyFailureAction::None
        );
        assert_eq!(
            config.action_for_failure(FailureKind::Timeout),
            ProxyFailureAction::Retry
        );
    }

    #[test]
    fn circuit_breaker_counts_configured_failure_kinds() {
        let config = ProxyCircuitBreakerConfig {
            enabled: true,
            count_network_errors: false,
            count_timeout_errors: true,
            count_server_errors: false,
            failure_status_codes: vec![429],
            ..Default::default()
        };

        assert!(!config.should_count_failure(FailureKind::Network));
        assert!(config.should_count_failure(FailureKind::Timeout));
        assert!(config.should_count_failure(FailureKind::Status(429)));
        assert!(!config.should_count_failure(FailureKind::Status(500)));
    }

    #[test]
    fn backend_health_recovers_after_cooldown_window() {
        let mut health = BackendHealth::default();
        let circuit_breaker = ProxyCircuitBreakerConfig {
            enabled: true,
            failure_threshold: 1,
            cooldown_seconds: 5,
            ..Default::default()
        };

        health.record_failure(
            100,
            &circuit_breaker,
            FailureKind::Timeout,
            "timeout".to_string(),
        );
        assert!(!health.is_healthy(100));
        assert!(!health.is_healthy(104));
        assert!(health.is_healthy(105));

        health.record_success();
        assert!(health.is_healthy(105));
        assert_eq!(health.consecutive_failures, 0);
    }
}
