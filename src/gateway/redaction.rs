use std::collections::HashSet;

use regex::{NoExpand, Regex};
use serde_json::Value;

use super::GatewayError;
use super::config::GatewayRedactionConfig;

#[derive(Debug, Clone, Copy, Default)]
struct RedactionContext {
    sanitize_query: bool,
}

#[derive(Debug)]
pub(crate) struct GatewayRedactor {
    replacement: String,
    redact_key_names: HashSet<String>,
    redact_query_params: HashSet<String>,
    sanitize_query_in_keys: HashSet<String>,
    redact_json_pointers: Vec<Vec<String>>,
    redact_regexes: Vec<Regex>,
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
                    replacement: "<redacted>".to_string(),
                    redact_key_names: HashSet::new(),
                    redact_query_params: HashSet::new(),
                    sanitize_query_in_keys: HashSet::new(),
                    redact_json_pointers: Vec::new(),
                    redact_regexes: Vec::new(),
                })
            }
        }
    }

    pub(crate) fn validate_config(config: &GatewayRedactionConfig) -> Result<(), GatewayError> {
        let _ = Self::try_new(config)?;
        Ok(())
    }

    fn try_new(config: &GatewayRedactionConfig) -> Result<Self, GatewayError> {
        let replacement = config.replacement.trim();
        if replacement.is_empty() {
            return Err(GatewayError::InvalidRequest {
                reason: "observability.redaction.replacement must not be empty".to_string(),
            });
        }

        let redact_key_names = normalize_list(
            config
                .redact_key_names
                .iter()
                .chain(config.redact_headers.iter()),
        );
        let redact_query_params = normalize_list(&config.redact_query_params);
        let sanitize_query_in_keys = normalize_list(&config.sanitize_query_in_keys);

        let mut redact_json_pointers = Vec::new();
        for pointer in &config.redact_json_pointers {
            let pointer = pointer.trim();
            if pointer.is_empty() {
                continue;
            }
            redact_json_pointers.push(parse_json_pointer(pointer)?);
        }

        let mut redact_regexes = Vec::new();
        for pattern in &config.redact_regexes {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                continue;
            }
            let regex = Regex::new(pattern).map_err(|err| GatewayError::InvalidRequest {
                reason: format!("invalid observability.redaction regex '{pattern}': {err}"),
            })?;
            redact_regexes.push(regex);
        }

        Ok(Self {
            replacement: replacement.to_string(),
            redact_key_names,
            redact_query_params,
            sanitize_query_in_keys,
            redact_json_pointers,
            redact_regexes,
        })
    }

    pub(crate) fn redact(&self, value: Value) -> Value {
        let mut value = value;
        self.redact_value_in_place(&mut value, RedactionContext::default());
        for pointer in &self.redact_json_pointers {
            redact_json_pointer_in_place(&mut value, pointer, &self.replacement);
        }
        value
    }

    fn redact_value_in_place(&self, value: &mut Value, ctx: RedactionContext) {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
            Value::String(value) => {
                if ctx.sanitize_query {
                    *value =
                        redact_query_string(&self.redact_query_params, &self.replacement, value);
                }
                if !self.redact_regexes.is_empty() {
                    let mut out = value.clone();
                    for regex in &self.redact_regexes {
                        if regex.is_match(&out) {
                            out = regex
                                .replace_all(&out, NoExpand(&self.replacement))
                                .to_string();
                        }
                    }
                    *value = out;
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.redact_value_in_place(item, ctx);
                }
            }
            Value::Object(map) => {
                for (key, value) in map {
                    let normalized_key = normalize_name(key);
                    if self.redact_key_names.contains(&normalized_key) {
                        *value = Value::String(self.replacement.clone());
                        continue;
                    }

                    let child_ctx = RedactionContext {
                        sanitize_query: ctx.sanitize_query
                            || self.sanitize_query_in_keys.contains(&normalized_key),
                    };
                    self.redact_value_in_place(value, child_ctx);
                }
            }
        }
    }
}

fn normalize_list<'a>(values: impl IntoIterator<Item = &'a String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for value in values {
        let normalized = normalize_name(value);
        if normalized.is_empty() {
            continue;
        }
        out.insert(normalized);
    }
    out
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn redact_query_string(redact: &HashSet<String>, replacement: &str, url: &str) -> String {
    let Some((path, query_and_fragment)) = url.split_once('?') else {
        return url.to_string();
    };
    let (query, fragment) = query_and_fragment
        .split_once('#')
        .map(|(query, fragment)| (query, Some(fragment)))
        .unwrap_or((query_and_fragment, None));

    let mut out = String::with_capacity(url.len());
    out.push_str(path);
    out.push('?');

    let mut first = true;
    for pair in query.split('&') {
        if !first {
            out.push('&');
        }
        first = false;

        let (key, _value) = pair.split_once('=').unwrap_or((pair, ""));
        let normalized_key = normalize_name(key);
        if redact.contains(&normalized_key) {
            out.push_str(key);
            out.push('=');
            out.push_str(replacement);
        } else {
            out.push_str(pair);
        }
    }

    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }

    out
}

fn parse_json_pointer(pointer: &str) -> Result<Vec<String>, GatewayError> {
    if pointer.is_empty() {
        return Err(GatewayError::InvalidRequest {
            reason: "observability.redaction.redact_json_pointers must not include empty pointers"
                .to_string(),
        });
    }
    if !pointer.starts_with('/') {
        return Err(GatewayError::InvalidRequest {
            reason: format!(
                "observability.redaction.redact_json_pointers must start with '/': {pointer}"
            ),
        });
    }

    let mut segments = Vec::new();
    for raw_segment in pointer.split('/').skip(1) {
        segments.push(decode_json_pointer_segment(raw_segment).map_err(|reason| {
            GatewayError::InvalidRequest {
                reason: format!(
                    "invalid observability.redaction json pointer segment '{raw_segment}': {reason}"
                ),
            }
        })?);
    }
    Ok(segments)
}

fn decode_json_pointer_segment(segment: &str) -> Result<String, &'static str> {
    if !segment.contains('~') {
        return Ok(segment.to_string());
    }

    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }

        let Some(next) = chars.next() else {
            return Err("dangling '~' escape");
        };
        match next {
            '0' => out.push('~'),
            '1' => out.push('/'),
            _ => return Err("invalid '~' escape (expected ~0 or ~1)"),
        }
    }
    Ok(out)
}

fn redact_json_pointer_in_place(value: &mut Value, pointer: &[String], replacement: &str) {
    if pointer.is_empty() {
        *value = Value::String(replacement.to_string());
        return;
    }

    let mut current = value;
    for segment in &pointer[..pointer.len().saturating_sub(1)] {
        match current {
            Value::Object(map) => {
                let Some(next) = map.get_mut(segment) else {
                    return;
                };
                current = next;
            }
            Value::Array(items) => {
                let Ok(idx) = segment.parse::<usize>() else {
                    return;
                };
                let Some(next) = items.get_mut(idx) else {
                    return;
                };
                current = next;
            }
            _ => return,
        }
    }

    let Some(last) = pointer.last() else {
        return;
    };
    match current {
        Value::Object(map) => {
            if map.contains_key(last) {
                map.insert(last.clone(), Value::String(replacement.to_string()));
            }
        }
        Value::Array(items) => {
            if let Ok(idx) = last.parse::<usize>() {
                if idx < items.len() {
                    items[idx] = Value::String(replacement.to_string());
                }
            }
        }
        _ => {}
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
