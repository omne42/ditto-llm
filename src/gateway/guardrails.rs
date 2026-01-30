use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    #[serde(default)]
    pub banned_phrases: Vec<String>,
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

        if self.banned_phrases.is_empty() {
            return Ok(());
        }

        let content = request.prompt.to_lowercase();
        for phrase in &self.banned_phrases {
            if phrase.is_empty() {
                continue;
            }
            if content.contains(&phrase.to_lowercase()) {
                return Err(GatewayError::GuardrailRejected {
                    reason: format!("banned_phrase:{phrase}"),
                });
            }
        }

        Ok(())
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
