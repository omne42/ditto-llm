use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest, VirtualKeyConfig};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterConfig {
    pub default_backend: String,
    pub rules: Vec<RouteRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteRule {
    pub model_prefix: String,
    pub backend: String,
}

impl RouteRule {
    pub fn matches(&self, model: &str) -> bool {
        model.starts_with(&self.model_prefix)
    }
}

#[derive(Debug)]
pub struct Router {
    config: RouterConfig,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        Self { config }
    }

    pub fn select_backend(
        &self,
        request: &GatewayRequest,
        key: &VirtualKeyConfig,
    ) -> Result<String, GatewayError> {
        if let Some(route) = &key.route {
            return Ok(route.clone());
        }

        for rule in &self.config.rules {
            if rule.matches(&request.model) {
                return Ok(rule.backend.clone());
            }
        }

        if self.config.default_backend.is_empty() {
            return Err(GatewayError::BackendNotFound {
                name: "default".to_string(),
            });
        }
        Ok(self.config.default_backend.clone())
    }
}
