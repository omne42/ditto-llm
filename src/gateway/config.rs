use serde::{Deserialize, Serialize};

use super::{
    BudgetConfig, CacheConfig, GuardrailsConfig, LimitsConfig, PassthroughConfig, RouterConfig,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub virtual_keys: Vec<VirtualKeyConfig>,
    pub router: RouterConfig,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VirtualKeyConfig {
    pub id: String,
    pub token: String,
    pub enabled: bool,
    pub limits: LimitsConfig,
    pub budget: BudgetConfig,
    pub cache: CacheConfig,
    pub guardrails: GuardrailsConfig,
    pub passthrough: PassthroughConfig,
    pub route: Option<String>,
}

impl std::fmt::Debug for VirtualKeyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualKeyConfig")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .field("enabled", &self.enabled)
            .field("limits", &self.limits)
            .field("budget", &self.budget)
            .field("cache", &self.cache)
            .field("guardrails", &self.guardrails)
            .field("passthrough", &self.passthrough)
            .field("route", &self.route)
            .finish()
    }
}

impl VirtualKeyConfig {
    pub fn new(id: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            token: token.into(),
            enabled: true,
            limits: LimitsConfig::default(),
            budget: BudgetConfig::default(),
            cache: CacheConfig::default(),
            guardrails: GuardrailsConfig::default(),
            passthrough: PassthroughConfig::default(),
            route: None,
        }
    }
}
