use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ProviderConfig;

use super::{
    BudgetConfig, CacheConfig, GuardrailsConfig, LimitsConfig, PassthroughConfig, RouterConfig,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
    pub virtual_keys: Vec<VirtualKeyConfig>,
    pub router: RouterConfig,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<ProviderConfig>,
    #[serde(default)]
    pub model_map: BTreeMap<String, String>,
}

impl std::fmt::Debug for BackendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendConfig")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("headers", &"<redacted>")
            .field("provider", &self.provider)
            .field("provider_config", &"<redacted>")
            .field("model_map", &self.model_map)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VirtualKeyConfig {
    pub id: String,
    pub token: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
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
            .field("project_id", &self.project_id)
            .field("user_id", &self.user_id)
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
            project_id: None,
            user_id: None,
            limits: LimitsConfig::default(),
            budget: BudgetConfig::default(),
            cache: CacheConfig::default(),
            guardrails: GuardrailsConfig::default(),
            passthrough: PassthroughConfig::default(),
            route: None,
        }
    }
}
