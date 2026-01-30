use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    #[serde(default)]
    pub banned_phrases: Vec<String>,
    #[serde(default)]
    pub banned_regexes: Vec<String>,
    #[serde(default)]
    pub block_pii: bool,
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
    #[serde(default)]
    pub allow_models: Vec<String>,
    #[serde(default)]
    pub deny_models: Vec<String>,
}

impl GuardrailsConfig {
    pub fn check(&self, request: &GatewayRequest) -> Result<(), GatewayError> {
        if let Some(reason) = self.check_model(&request.model) {
            return Err(GatewayError::GuardrailRejected { reason });
        }

        if let Some(limit) = self.max_input_tokens {
            if request.input_tokens > limit {
                return Err(GatewayError::GuardrailRejected {
                    reason: format!("input_tokens>{limit}"),
                });
            }
        }

        if let Some(reason) = self.check_text(&request.prompt) {
            return Err(GatewayError::GuardrailRejected { reason });
        }

        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        for raw in &self.banned_regexes {
            let pattern = raw.trim();
            if pattern.is_empty() {
                continue;
            }
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|err| format!("invalid banned_regex {pattern}: {err}"))?;
        }
        Ok(())
    }

    pub fn has_text_filters(&self) -> bool {
        !self.banned_phrases.is_empty() || !self.banned_regexes.is_empty() || self.block_pii
    }

    pub fn check_text(&self, text: &str) -> Option<String> {
        if !self.banned_phrases.is_empty() {
            let content = text.to_lowercase();
            for phrase in &self.banned_phrases {
                let phrase = phrase.trim();
                if phrase.is_empty() {
                    continue;
                }
                if content.contains(&phrase.to_lowercase()) {
                    return Some(format!("banned_phrase:{phrase}"));
                }
            }
        }

        if !self.banned_regexes.is_empty() {
            for raw in &self.banned_regexes {
                let pattern = raw.trim();
                if pattern.is_empty() {
                    continue;
                }
                let regex = match RegexBuilder::new(pattern).case_insensitive(true).build() {
                    Ok(regex) => regex,
                    Err(_) => return Some(format!("banned_regex_invalid:{pattern}")),
                };
                if regex.is_match(text) {
                    return Some(format!("banned_regex:{pattern}"));
                }
            }
        }

        if self.block_pii {
            if email_pii_regex().is_match(text) {
                return Some("pii:email".to_string());
            }
            if ssn_pii_regex().is_match(text) {
                return Some("pii:ssn".to_string());
            }
        }

        None
    }

    pub fn check_model(&self, model: &str) -> Option<String> {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }

        for pattern in &self.deny_models {
            if model_matches_pattern(model, pattern) {
                return Some(format!("deny_model:{pattern}"));
            }
        }

        if !self.allow_models.is_empty()
            && !self
                .allow_models
                .iter()
                .any(|pattern| model_matches_pattern(model, pattern))
        {
            return Some(format!("model_not_allowed:{model}"));
        }

        None
    }
}

fn model_matches_pattern(model: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return model.starts_with(prefix);
    }

    model == pattern
}

fn email_pii_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        RegexBuilder::new(r"\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b")
            .case_insensitive(true)
            .build()
            .expect("email regex is valid")
    })
}

fn ssn_pii_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("ssn regex is valid"))
}
