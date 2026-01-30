use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Env, ProviderConfig};

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

impl GatewayConfig {
    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        for backend in &mut self.backends {
            backend.resolve_env(env)?;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<ProviderConfig>,
    #[serde(default)]
    pub model_map: BTreeMap<String, String>,
}

impl BackendConfig {
    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        self.base_url = expand_env_placeholders(&self.base_url, env)?;
        for value in self.headers.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        for value in self.query_params.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for BackendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendConfig")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("headers", &"<redacted>")
            .field("query_params", &"<redacted>")
            .field("provider", &self.provider)
            .field("provider_config", &"<redacted>")
            .field("model_map", &self.model_map)
            .finish()
    }
}

fn expand_env_placeholders(value: &str, env: &Env) -> Result<String, super::GatewayError> {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut idx = 0;
    let mut last = 0;

    while idx < bytes.len() {
        if bytes[idx] != b'$' || idx + 1 >= bytes.len() || bytes[idx + 1] != b'{' {
            idx += 1;
            continue;
        }

        let placeholder_start = idx + 2;
        let mut end = None;
        for (pos, byte) in bytes[placeholder_start..].iter().copied().enumerate() {
            if byte == b'}' {
                end = Some(placeholder_start + pos);
                break;
            }
        }

        let Some(placeholder_end) = end else {
            return Err(super::GatewayError::InvalidRequest {
                reason: "unterminated env placeholder".to_string(),
            });
        };

        let name = &value[placeholder_start..placeholder_end];
        let name = name.trim();
        if name.is_empty() {
            return Err(super::GatewayError::InvalidRequest {
                reason: "empty env placeholder".to_string(),
            });
        }

        let resolved = env
            .get(name)
            .ok_or_else(|| super::GatewayError::InvalidRequest {
                reason: format!("missing env var: {name}"),
            })?;

        if resolved.trim().is_empty() {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!("env var is empty: {name}"),
            });
        }

        out.push_str(&value[last..idx]);
        out.push_str(&resolved);
        idx = placeholder_end + 1;
        last = idx;
    }

    if last < value.len() {
        out.push_str(&value[last..]);
    }
    Ok(out)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_budget: Option<BudgetConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_budget: Option<BudgetConfig>,
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
            .field("project_budget", &self.project_budget)
            .field("user_budget", &self.user_budget)
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
            project_budget: None,
            user_budget: None,
            limits: LimitsConfig::default(),
            budget: BudgetConfig::default(),
            cache: CacheConfig::default(),
            guardrails: GuardrailsConfig::default(),
            passthrough: PassthroughConfig::default(),
            route: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_config_resolves_env_placeholders() {
        let env = Env {
            dotenv: BTreeMap::from([
                ("OPENAI_API_KEY".to_string(), "sk-test".to_string()),
                ("API_VERSION".to_string(), "2024-01-01".to_string()),
            ]),
        };

        let mut backend = BackendConfig {
            name: "primary".to_string(),
            base_url: "https://example.com/${OPENAI_API_KEY}".to_string(),
            headers: BTreeMap::from([(
                "authorization".to_string(),
                "Bearer ${OPENAI_API_KEY}".to_string(),
            )]),
            query_params: BTreeMap::from([(
                "api-version".to_string(),
                "${API_VERSION}".to_string(),
            )]),
            provider: None,
            provider_config: None,
            model_map: BTreeMap::new(),
        };

        backend.resolve_env(&env).expect("resolve");
        assert_eq!(backend.base_url, "https://example.com/sk-test");
        assert_eq!(
            backend.headers.get("authorization").map(|s| s.as_str()),
            Some("Bearer sk-test")
        );
        assert_eq!(
            backend.query_params.get("api-version").map(|s| s.as_str()),
            Some("2024-01-01")
        );
    }

    #[test]
    fn backend_config_errors_when_env_missing() {
        let env = Env {
            dotenv: BTreeMap::from([("OPENAI_API_KEY".to_string(), " ".to_string())]),
        };

        let mut backend = BackendConfig {
            name: "primary".to_string(),
            base_url: "https://example.com".to_string(),
            headers: BTreeMap::from([(
                "authorization".to_string(),
                "Bearer ${OPENAI_API_KEY}".to_string(),
            )]),
            query_params: BTreeMap::new(),
            provider: None,
            provider_config: None,
            model_map: BTreeMap::new(),
        };

        let err = backend.resolve_env(&env).expect_err("missing env");
        assert!(err.to_string().contains("env var is empty: OPENAI_API_KEY"));
    }
}
