use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    pub banned_phrases: Vec<String>,
    pub max_input_tokens: Option<u32>,
}

impl GuardrailsConfig {
    pub fn check(&self, request: &GatewayRequest) -> Result<(), GatewayError> {
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
}
