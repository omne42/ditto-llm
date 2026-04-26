use std::sync::Arc;

use redaction_kit::stable_sample_json_payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::GatewaySamplingConfig;
use super::redaction::GatewayRedactor;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservabilitySnapshot {
    pub requests: u64,
    pub cache_hits: u64,
    pub rate_limited: u64,
    pub guardrail_blocked: u64,
    pub budget_exceeded: u64,
    pub backend_calls: u64,
}

#[derive(Debug, Default)]
pub struct Observability {
    snapshot: ObservabilitySnapshot,
}

impl Observability {
    pub fn record_request(&mut self) {
        self.snapshot.requests = self.snapshot.requests.saturating_add(1);
    }

    pub fn record_cache_hit(&mut self) {
        self.snapshot.cache_hits = self.snapshot.cache_hits.saturating_add(1);
    }

    pub fn record_rate_limited(&mut self) {
        self.snapshot.rate_limited = self.snapshot.rate_limited.saturating_add(1);
    }

    pub fn record_guardrail_blocked(&mut self) {
        self.snapshot.guardrail_blocked = self.snapshot.guardrail_blocked.saturating_add(1);
    }

    pub fn record_budget_exceeded(&mut self) {
        self.snapshot.budget_exceeded = self.snapshot.budget_exceeded.saturating_add(1);
    }

    pub fn record_backend_call(&mut self) {
        self.snapshot.backend_calls = self.snapshot.backend_calls.saturating_add(1);
    }

    pub fn snapshot(&self) -> ObservabilitySnapshot {
        self.snapshot.clone()
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GatewayObservabilitySink {
    JsonLogs,
    Audit,
    Devtools,
}

impl GatewayObservabilitySink {
    fn as_str(self) -> &'static str {
        match self {
            Self::JsonLogs => "json_logs",
            Self::Audit => "audit",
            Self::Devtools => "devtools",
        }
    }
}

#[derive(Debug)]
pub(crate) struct GatewayObservabilityPolicy {
    redactor: Arc<GatewayRedactor>,
    sampling: GatewaySamplingPolicy,
}

impl GatewayObservabilityPolicy {
    pub(crate) fn new(redactor: Arc<GatewayRedactor>, config: &GatewaySamplingConfig) -> Self {
        Self {
            redactor,
            sampling: GatewaySamplingPolicy::from_config(config),
        }
    }

    pub(crate) fn prepare_event(
        &self,
        sink: GatewayObservabilitySink,
        payload: Value,
    ) -> Option<Value> {
        if !self.sampling.should_emit(sink, &payload) {
            return None;
        }
        Some(self.redactor.redact(payload))
    }

    #[allow(dead_code)]
    pub(crate) fn redact_metric_label(&self, label_name: &str, value: &str) -> String {
        self.redactor.redact_named_string(label_name, value)
    }

    #[allow(dead_code)]
    pub(crate) fn redact_prometheus_render(&self, rendered: &str) -> String {
        self.redactor.redact_prometheus_render(rendered)
    }
}

#[derive(Clone, Copy, Debug)]
struct GatewaySamplingPolicy {
    json_logs_rate: f64,
    audit_rate: f64,
    devtools_rate: f64,
}

impl GatewaySamplingPolicy {
    fn from_config(config: &GatewaySamplingConfig) -> Self {
        Self {
            json_logs_rate: config.json_logs_rate,
            audit_rate: config.audit_rate,
            devtools_rate: config.devtools_rate,
        }
    }

    fn should_emit(&self, sink: GatewayObservabilitySink, payload: &Value) -> bool {
        let rate = match sink {
            GatewayObservabilitySink::JsonLogs => self.json_logs_rate,
            GatewayObservabilitySink::Audit => self.audit_rate,
            GatewayObservabilitySink::Devtools => self.devtools_rate,
        };

        if rate <= 0.0 {
            return false;
        }
        if rate >= 1.0 {
            return true;
        }

        stable_sample_json_payload(sink.as_str(), payload, rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{GatewayRedactionConfig, GatewaySamplingConfig};
    use serde_json::json;

    fn policy(
        redaction: GatewayRedactionConfig,
        sampling: GatewaySamplingConfig,
    ) -> GatewayObservabilityPolicy {
        GatewayObservabilityPolicy::new(
            Arc::new(GatewayRedactor::from_config(&redaction)),
            &sampling,
        )
    }

    #[test]
    fn sink_sampling_is_configurable() {
        let policy = policy(
            GatewayRedactionConfig::default(),
            GatewaySamplingConfig {
                json_logs_rate: 0.0,
                audit_rate: 1.0,
                devtools_rate: 0.0,
            },
        );
        let payload = json!({"request_id": "req-1", "token": "sk-secret-1234567890"});

        assert!(
            policy
                .prepare_event(GatewayObservabilitySink::JsonLogs, payload.clone())
                .is_none()
        );
        assert!(
            policy
                .prepare_event(GatewayObservabilitySink::Devtools, payload.clone())
                .is_none()
        );

        let audit = policy
            .prepare_event(GatewayObservabilitySink::Audit, payload)
            .expect("audit payload");
        assert_eq!(audit["token"].as_str(), Some("<redacted>"));
    }

    #[test]
    fn sampling_is_stable_for_same_request_id() {
        let policy = policy(
            GatewayRedactionConfig::default(),
            GatewaySamplingConfig {
                json_logs_rate: 0.5,
                audit_rate: 1.0,
                devtools_rate: 1.0,
            },
        );

        let first = policy
            .prepare_event(
                GatewayObservabilitySink::JsonLogs,
                json!({"request_id": "req-stable", "step": "request"}),
            )
            .is_some();
        let second = policy
            .prepare_event(
                GatewayObservabilitySink::JsonLogs,
                json!({"request_id": "req-stable", "step": "response", "safe": true}),
            )
            .is_some();

        assert_eq!(first, second);
    }

    #[test]
    fn metric_label_redaction_reuses_redaction_rules() {
        let policy = policy(
            GatewayRedactionConfig::default(),
            GatewaySamplingConfig::default(),
        );

        assert_eq!(
            policy.redact_metric_label("path", "/v1/chat/completions?token=abc&safe=1"),
            "/v1/chat/completions?token=<redacted>&safe=1"
        );
        assert_eq!(
            policy.redact_metric_label("model", "sk-1234567890"),
            "<redacted>"
        );
    }

    #[test]
    fn invalid_sampling_rate_is_rejected() {
        let err = GatewaySamplingConfig {
            json_logs_rate: 1.5,
            audit_rate: 1.0,
            devtools_rate: 1.0,
        }
        .validate()
        .expect_err("invalid rate should fail");

        let crate::gateway::GatewayError::InvalidRequest { reason } = err else {
            panic!("unexpected error: {err:?}");
        };
        assert!(reason.contains("json_logs_rate"));
    }

    #[test]
    fn prometheus_render_redacts_label_values() {
        let policy = policy(
            GatewayRedactionConfig {
                redact_key_names: vec!["virtual_key_id".to_string()],
                ..GatewayRedactionConfig::default()
            },
            GatewaySamplingConfig::default(),
        );
        let rendered = concat!(
            "metric_without_labels 1\n",
            "metric_with_labels{virtual_key_id=\"vk-secret\",model=\"sk-1234567890\",path=\"/v1/chat/completions?token=abc\"} 1\n",
        );

        let out = policy.redact_prometheus_render(rendered);
        assert!(out.contains("metric_without_labels 1\n"));
        assert!(out.contains(
            "metric_with_labels{virtual_key_id=\"<redacted>\",model=\"<redacted>\",path=\"/v1/chat/completions?token=<redacted>\"} 1\n"
        ));
    }
}
