use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PassthroughConfig {
    pub allow: bool,
    pub bypass_cache: bool,
}

impl Default for PassthroughConfig {
    fn default() -> Self {
        Self {
            allow: true,
            bypass_cache: true,
        }
    }
}

impl PassthroughConfig {
    pub fn validate(&self, request: &GatewayRequest) -> Result<(), GatewayError> {
        if request.passthrough && !self.allow {
            return Err(GatewayError::GuardrailRejected {
                reason: "passthrough_disabled".to_string(),
            });
        }
        Ok(())
    }

    pub fn bypass_cache(&self, request: &GatewayRequest) -> bool {
        request.passthrough && self.allow && self.bypass_cache
    }
}
