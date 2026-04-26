use redaction_kit::{RedactionRules, Redactor};
use serde_json::Value;

use super::GatewayError;
use super::config::GatewayRedactionConfig;

#[derive(Debug)]
pub(crate) struct GatewayRedactor {
    inner: Redactor,
}

impl Default for GatewayRedactor {
    fn default() -> Self {
        Self::from_config(&GatewayRedactionConfig::default())
    }
}

impl GatewayRedactor {
    pub(crate) fn from_config(config: &GatewayRedactionConfig) -> Self {
        match Self::try_new(config) {
            Ok(redactor) => redactor,
            Err(err) => {
                eprintln!("invalid observability redaction config: {err}");
                Self::try_new(&GatewayRedactionConfig::default()).unwrap_or_else(|_| Self {
                    inner: Redactor::new(&RedactionRules::default())
                        .expect("default redaction rules must be valid"),
                })
            }
        }
    }

    pub(crate) fn validate_config(config: &GatewayRedactionConfig) -> Result<(), GatewayError> {
        let _ = Self::try_new(config)?;
        Ok(())
    }

    fn try_new(config: &GatewayRedactionConfig) -> Result<Self, GatewayError> {
        let rules = redaction_rules_from_config(config);
        let inner = Redactor::new(&rules).map_err(redaction_config_error)?;
        Ok(Self { inner })
    }

    pub(crate) fn redact(&self, value: Value) -> Value {
        self.inner.redact_json_value(value)
    }

    #[allow(dead_code)]
    pub(crate) fn redact_named_string(&self, key: &str, value: &str) -> String {
        self.inner.redact_named_string(key, value)
    }

    #[allow(dead_code)]
    pub(crate) fn redact_prometheus_render(&self, rendered: &str) -> String {
        self.inner.redact_prometheus_render(rendered)
    }
}

fn redaction_rules_from_config(config: &GatewayRedactionConfig) -> RedactionRules {
    let mut redact_key_names = config.redact_key_names.clone();
    redact_key_names.extend(config.redact_headers.clone());

    RedactionRules {
        replacement: config.replacement.clone(),
        redact_key_names,
        redact_query_params: config.redact_query_params.clone(),
        sanitize_query_in_keys: config.sanitize_query_in_keys.clone(),
        redact_json_pointers: config.redact_json_pointers.clone(),
        redact_regexes: config.redact_regexes.clone(),
    }
}

fn redaction_config_error(err: redaction_kit::RedactionError) -> GatewayError {
    GatewayError::InvalidRequest {
        reason: format!("invalid observability.redaction config: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_values_by_key_name() {
        let redactor = GatewayRedactor::default();
        let input = json!({
            "authorization": "Bearer sk-test",
            "nested": {
                "token": "sk-1234567890",
                "safe": "ok"
            },
            "items": [
                { "api_key": "k" }
            ]
        });

        let out = redactor.redact(input);
        assert_eq!(out["authorization"].as_str(), Some("<redacted>"));
        assert_eq!(out["nested"]["token"].as_str(), Some("<redacted>"));
        assert_eq!(out["nested"]["safe"].as_str(), Some("ok"));
        assert_eq!(out["items"][0]["api_key"].as_str(), Some("<redacted>"));
    }

    #[test]
    fn redacts_query_params_for_path_like_keys() {
        let redactor = GatewayRedactor::default();
        let input = json!({
            "path": "/v1/chat/completions?api_key=abc&x=1#frag",
            "url": "http://example.test/?token=abc&ok=1",
            "other": "http://example.test/?api_key=abc&x=1"
        });

        let out = redactor.redact(input);
        assert_eq!(
            out["path"].as_str(),
            Some("/v1/chat/completions?api_key=<redacted>&x=1#frag")
        );
        assert_eq!(
            out["url"].as_str(),
            Some("http://example.test/?token=<redacted>&ok=1")
        );
        assert_eq!(
            out["other"].as_str(),
            Some("http://example.test/?api_key=abc&x=1")
        );
    }

    #[test]
    fn redacts_regexes_in_strings() {
        let redactor = GatewayRedactor::default();
        let input = json!({
            "message": "Authorization: Bearer sk-test",
            "another": "sk-1234567890",
            "safe": "hello"
        });

        let out = redactor.redact(input);
        assert_eq!(out["message"].as_str(), Some("Authorization: <redacted>"));
        assert_eq!(out["another"].as_str(), Some("<redacted>"));
        assert_eq!(out["safe"].as_str(), Some("hello"));
    }

    #[test]
    fn redacts_json_pointers() {
        let config = GatewayRedactionConfig {
            redact_json_pointers: vec!["/a/b/0/c".to_string()],
            ..GatewayRedactionConfig::default()
        };
        GatewayRedactor::validate_config(&config).expect("validate config");

        let redactor = GatewayRedactor::from_config(&config);
        let input = json!({
            "a": {
                "b": [
                    { "c": "secret", "d": "keep" }
                ]
            }
        });

        let out = redactor.redact(input);
        assert_eq!(out["a"]["b"][0]["c"].as_str(), Some("<redacted>"));
        assert_eq!(out["a"]["b"][0]["d"].as_str(), Some("keep"));
    }

    #[test]
    fn validate_rejects_empty_replacement() {
        let config = GatewayRedactionConfig {
            replacement: "  ".to_string(),
            ..GatewayRedactionConfig::default()
        };

        let err = GatewayRedactor::validate_config(&config).expect_err("expected error");
        let GatewayError::InvalidRequest { reason } = err else {
            panic!("unexpected error: {err:?}");
        };
        assert!(reason.contains("replacement"));
    }
}
