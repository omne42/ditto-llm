use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

    pub(crate) fn redact_metric_label(&self, label_name: &str, value: &str) -> String {
        self.redactor.redact_named_string(label_name, value)
    }

    pub(crate) fn redact_prometheus_render(&self, rendered: &str) -> String {
        if rendered.is_empty() {
            return String::new();
        }

        let has_trailing_newline = rendered.ends_with('\n');
        let mut out = String::with_capacity(rendered.len());
        for line in rendered.lines() {
            out.push_str(&self.redact_prometheus_line(line));
            out.push('\n');
        }
        if !has_trailing_newline {
            let _ = out.pop();
        }
        out
    }

    fn redact_prometheus_line(&self, line: &str) -> String {
        if line.starts_with('#') {
            return line.to_string();
        }
        let Some(open_idx) = line.find('{') else {
            return line.to_string();
        };
        let Some(close_rel) = line[open_idx + 1..].find('}') else {
            return line.to_string();
        };
        let close_idx = open_idx + 1 + close_rel;
        let Some(redacted_labels) = self.redact_prometheus_labels(&line[open_idx + 1..close_idx])
        else {
            return line.to_string();
        };

        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..open_idx + 1]);
        out.push_str(&redacted_labels);
        out.push_str(&line[close_idx..]);
        out
    }

    fn redact_prometheus_labels(&self, labels: &str) -> Option<String> {
        let mut out = String::with_capacity(labels.len());
        let bytes = labels.as_bytes();
        let mut idx = 0usize;

        while idx < bytes.len() {
            let name_start = idx;
            while idx < bytes.len() && bytes[idx] != b'=' {
                idx += 1;
            }
            if idx == bytes.len() || idx + 1 >= bytes.len() || bytes[idx + 1] != b'"' {
                return None;
            }
            let name = &labels[name_start..idx];
            idx += 2;

            let mut raw_value = String::new();
            let mut closed = false;
            while idx < bytes.len() {
                match bytes[idx] {
                    b'\\' => {
                        idx += 1;
                        if idx == bytes.len() {
                            return None;
                        }
                        match bytes[idx] {
                            b'\\' => raw_value.push('\\'),
                            b'"' => raw_value.push('"'),
                            b'n' => raw_value.push('\n'),
                            other => raw_value.push(other as char),
                        }
                        idx += 1;
                    }
                    b'"' => {
                        idx += 1;
                        closed = true;
                        break;
                    }
                    byte => {
                        raw_value.push(byte as char);
                        idx += 1;
                    }
                }
            }
            if !closed {
                return None;
            }

            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(name);
            out.push_str("=\"");
            out.push_str(&escape_prometheus_label_value(
                &self.redact_metric_label(name, &raw_value),
            ));
            out.push('"');

            if idx == bytes.len() {
                break;
            }
            if bytes[idx] != b',' {
                return None;
            }
            idx += 1;
        }

        Some(out)
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

        let threshold = (rate * u64::MAX as f64).floor() as u64;
        sample_hash(sink, payload) <= threshold
    }
}

fn sample_hash(sink: GatewayObservabilitySink, payload: &Value) -> u64 {
    let mut hash = fnv1a64_init();
    hash = fnv1a64_update(hash, sink.as_str().as_bytes());
    hash = fnv1a64_update(hash, b"|");

    if let Some(identity) = find_sampling_identity(payload) {
        return fnv1a64_update(hash, identity.as_bytes());
    }

    match serde_json::to_vec(payload) {
        Ok(bytes) => fnv1a64_update(hash, &bytes),
        Err(_) => hash,
    }
}

fn find_sampling_identity(value: &Value) -> Option<String> {
    const PREFERRED_KEYS: &[&str] = &["request_id", "trace_id", "response_id", "session_id", "id"];

    match value {
        Value::Object(map) => find_sampling_identity_in_object(map, PREFERRED_KEYS),
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_sampling_identity(item) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_sampling_identity_in_object(
    map: &Map<String, Value>,
    preferred_keys: &[&str],
) -> Option<String> {
    for key in preferred_keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        let Some(value) = value.as_str() else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    for value in map.values() {
        if let Some(found) = find_sampling_identity(value) {
            return Some(found);
        }
    }

    None
}

const FNV1A64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV1A64_PRIME: u64 = 0x100000001b3;

fn fnv1a64_init() -> u64 {
    FNV1A64_OFFSET_BASIS
}

fn fnv1a64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}

fn escape_prometheus_label_value(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
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
