use std::collections::{BTreeMap, HashSet};

use omne_integrity_primitives::hash_sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ditto_core::config::{Env, ProviderConfig};
use secret_kit::spec::resolve_secret;

use super::{
    BudgetConfig, CacheConfig, GuardrailsConfig, LimitsConfig, PassthroughConfig, RouterConfig,
};

pub(crate) const VIRTUAL_KEY_TOKEN_HASH_PREFIX: &str = "sha256:";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GatewayObservabilityConfig {
    #[serde(default)]
    pub redaction: GatewayRedactionConfig,
    #[serde(default)]
    pub sampling: GatewaySamplingConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewaySamplingConfig {
    #[serde(default = "default_observability_sample_rate")]
    pub json_logs_rate: f64,
    #[serde(default = "default_observability_sample_rate")]
    pub audit_rate: f64,
    #[serde(default = "default_observability_sample_rate")]
    pub devtools_rate: f64,
}

impl Default for GatewaySamplingConfig {
    fn default() -> Self {
        Self {
            json_logs_rate: default_observability_sample_rate(),
            audit_rate: default_observability_sample_rate(),
            devtools_rate: default_observability_sample_rate(),
        }
    }
}

impl GatewaySamplingConfig {
    pub fn validate(&self) -> Result<(), super::GatewayError> {
        validate_sample_rate("observability.sampling.json_logs_rate", self.json_logs_rate)?;
        validate_sample_rate("observability.sampling.audit_rate", self.audit_rate)?;
        validate_sample_rate("observability.sampling.devtools_rate", self.devtools_rate)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayRedactionConfig {
    #[serde(default = "default_redaction_replacement")]
    pub replacement: String,
    #[serde(default = "default_redact_headers")]
    pub redact_headers: Vec<String>,
    #[serde(default = "default_redact_key_names")]
    pub redact_key_names: Vec<String>,
    #[serde(default = "default_redact_query_params")]
    pub redact_query_params: Vec<String>,
    #[serde(default)]
    pub redact_json_pointers: Vec<String>,
    #[serde(default = "default_redact_regexes")]
    pub redact_regexes: Vec<String>,
    #[serde(default = "default_sanitize_query_in_keys")]
    pub sanitize_query_in_keys: Vec<String>,
}

impl Default for GatewayRedactionConfig {
    fn default() -> Self {
        Self {
            replacement: default_redaction_replacement(),
            redact_headers: default_redact_headers(),
            redact_key_names: default_redact_key_names(),
            redact_query_params: default_redact_query_params(),
            redact_json_pointers: Vec::new(),
            redact_regexes: default_redact_regexes(),
            sanitize_query_in_keys: default_sanitize_query_in_keys(),
        }
    }
}

impl GatewayRedactionConfig {
    pub fn validate(&self) -> Result<(), super::GatewayError> {
        super::redaction::GatewayRedactor::validate_config(self)
    }
}

fn default_observability_sample_rate() -> f64 {
    1.0
}

fn validate_sample_rate(name: &str, value: f64) -> Result<(), super::GatewayError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(super::GatewayError::InvalidRequest {
        reason: format!("{name} must be a finite value between 0.0 and 1.0"),
    })
}

fn default_redaction_replacement() -> String {
    "<redacted>".to_string()
}

fn default_redact_headers() -> Vec<String> {
    vec![
        "authorization".to_string(),
        "proxy-authorization".to_string(),
        "x-api-key".to_string(),
        "x-litellm-api-key".to_string(),
        "x-admin-token".to_string(),
        "x-ditto-virtual-key".to_string(),
    ]
}

fn default_redact_key_names() -> Vec<String> {
    vec![
        "virtual_key".to_string(),
        "api_key".to_string(),
        "apikey".to_string(),
        "token".to_string(),
        "access_token".to_string(),
        "refresh_token".to_string(),
        "client_secret".to_string(),
        "secret".to_string(),
        "password".to_string(),
        "session_token".to_string(),
    ]
}

fn default_redact_query_params() -> Vec<String> {
    vec![
        "api_key".to_string(),
        "api-key".to_string(),
        "key".to_string(),
        "token".to_string(),
        "access_token".to_string(),
        "refresh_token".to_string(),
        "authorization".to_string(),
    ]
}

fn default_sanitize_query_in_keys() -> Vec<String> {
    vec![
        "path".to_string(),
        "url".to_string(),
        "base_url".to_string(),
        "endpoint".to_string(),
    ]
}

fn default_redact_regexes() -> Vec<String> {
    vec![
        "(?i)bearer\\s+[^\\s]+".to_string(),
        "sk-[A-Za-z0-9]{10,}".to_string(),
    ]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
    pub virtual_keys: Vec<VirtualKeyConfig>,
    pub router: RouterConfig,
    #[serde(default)]
    pub a2a_agents: Vec<A2aAgentConfig>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub observability: GatewayObservabilityConfig,
}

impl GatewayConfig {
    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        for backend in &mut self.backends {
            backend.resolve_env(env)?;
        }
        for key in &mut self.virtual_keys {
            key.resolve_env(env)?;
        }
        for agent in &mut self.a2a_agents {
            agent.resolve_env(env)?;
        }
        for server in &mut self.mcp_servers {
            server.resolve_env(env)?;
        }
        Ok(())
    }

    pub async fn resolve_secrets(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        for backend in &mut self.backends {
            backend.resolve_secrets(env).await?;
        }
        for key in &mut self.virtual_keys {
            key.resolve_secrets(env).await?;
        }
        for agent in &mut self.a2a_agents {
            agent.resolve_secrets(env).await?;
        }
        for server in &mut self.mcp_servers {
            server.resolve_secrets(env).await?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), super::GatewayError> {
        let backend_names = self
            .backends
            .iter()
            .map(|backend| backend.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<HashSet<_>>();
        self.validate_with_backend_names(&backend_names)
    }

    pub(crate) fn validate_with_backend_names(
        &self,
        backend_names: &HashSet<String>,
    ) -> Result<(), super::GatewayError> {
        self.observability.redaction.validate()?;
        self.observability.sampling.validate()?;
        validate_virtual_key_configs(&self.virtual_keys)?;
        validate_virtual_key_routes(&self.virtual_keys, backend_names)?;
        validate_guardrails_config(&self.virtual_keys, &self.router)?;
        validate_router_against_backends(&self.router, backend_names)?;
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct A2aAgentConfig {
    pub agent_id: String,
    #[serde(default)]
    pub agent_card_params: Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl A2aAgentConfig {
    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        if let Value::Object(obj) = &mut self.agent_card_params
            && let Some(Value::String(url)) = obj.get_mut("url")
        {
            *url = expand_env_placeholders(url, env)?;
        }
        for value in self.headers.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        for value in self.query_params.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        Ok(())
    }

    pub async fn resolve_secrets(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        if let Value::Object(obj) = &mut self.agent_card_params
            && let Some(Value::String(url)) = obj.get_mut("url")
        {
            resolve_secret_in_string(url, env, "a2a_agents[].agent_card_params.url").await?;
        }
        for value in self.headers.values_mut() {
            resolve_secret_in_string(value, env, "a2a_agents[].headers").await?;
        }
        for value in self.query_params.values_mut() {
            resolve_secret_in_string(value, env, "a2a_agents[].query_params").await?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for A2aAgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A2aAgentConfig")
            .field("agent_id", &self.agent_id)
            .field("agent_card_params", &self.agent_card_params)
            .field("headers", &"<redacted>")
            .field("query_params", &"<redacted>")
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub server_id: String,
    #[serde(default, alias = "http_url")]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl McpServerConfig {
    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        self.url = expand_env_placeholders(&self.url, env)?;
        for value in self.headers.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        for value in self.query_params.values_mut() {
            *value = expand_env_placeholders(value, env)?;
        }
        Ok(())
    }

    pub async fn resolve_secrets(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        resolve_secret_in_string(&mut self.url, env, "mcp_servers[].url").await?;
        for value in self.headers.values_mut() {
            resolve_secret_in_string(value, env, "mcp_servers[].headers").await?;
        }
        for value in self.query_params.values_mut() {
            resolve_secret_in_string(value, env, "mcp_servers[].query_params").await?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerConfig")
            .field("server_id", &self.server_id)
            .field("url", &self.url)
            .field("headers", &"<redacted>")
            .field("query_params", &"<redacted>")
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_in_flight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
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
        if let Some(provider_config) = self.provider_config.as_mut() {
            if let Some(base_url) = provider_config.base_url.as_mut() {
                *base_url = expand_env_placeholders(base_url, env)?;
            }
            if let Some(default_model) = provider_config.default_model.as_mut() {
                *default_model = expand_env_placeholders(default_model, env)?;
            }
            if let Some(normalize_endpoint) = provider_config.normalize_endpoint.as_mut() {
                *normalize_endpoint = expand_env_placeholders(normalize_endpoint, env)?;
            }
            for value in provider_config.model_whitelist.iter_mut() {
                *value = expand_env_placeholders(value, env)?;
            }
            for value in provider_config.http_headers.values_mut() {
                *value = expand_env_placeholders(value, env)?;
            }
            for value in provider_config.http_query_params.values_mut() {
                *value = expand_env_placeholders(value, env)?;
            }
        }
        Ok(())
    }

    pub async fn resolve_secrets(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        resolve_secret_in_string(&mut self.base_url, env, "backends[].base_url").await?;
        for value in self.headers.values_mut() {
            resolve_secret_in_string(value, env, "backends[].headers").await?;
        }
        for value in self.query_params.values_mut() {
            resolve_secret_in_string(value, env, "backends[].query_params").await?;
        }

        if let Some(provider_config) = self.provider_config.as_mut() {
            if let Some(base_url) = provider_config.base_url.as_mut() {
                resolve_secret_in_string(base_url, env, "backends[].provider_config.base_url")
                    .await?;
            }
            if let Some(default_model) = provider_config.default_model.as_mut() {
                resolve_secret_in_string(
                    default_model,
                    env,
                    "backends[].provider_config.default_model",
                )
                .await?;
            }
            if let Some(normalize_endpoint) = provider_config.normalize_endpoint.as_mut() {
                resolve_secret_in_string(
                    normalize_endpoint,
                    env,
                    "backends[].provider_config.normalize_endpoint",
                )
                .await?;
            }
            for value in provider_config.model_whitelist.iter_mut() {
                resolve_secret_in_string(value, env, "backends[].provider_config.model_whitelist")
                    .await?;
            }
            for value in provider_config.http_headers.values_mut() {
                resolve_secret_in_string(value, env, "backends[].provider_config.http_headers")
                    .await?;
            }
            for value in provider_config.http_query_params.values_mut() {
                resolve_secret_in_string(
                    value,
                    env,
                    "backends[].provider_config.http_query_params",
                )
                .await?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for BackendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendConfig")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("max_in_flight", &self.max_in_flight)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("headers", &"<redacted>")
            .field("query_params", &"<redacted>")
            .field("provider", &self.provider)
            .field("provider_config", &"<redacted>")
            .field("model_map", &self.model_map)
            .finish()
    }
}

fn expand_env_placeholders(value: &str, env: &Env) -> Result<String, super::GatewayError> {
    let trimmed = value.trim();
    if let Some(name) = trimmed.strip_prefix("os.environ/") {
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

        return Ok(resolved);
    }

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
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_budget: Option<BudgetConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_budget: Option<BudgetConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_budget: Option<BudgetConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_limits: Option<LimitsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_limits: Option<LimitsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_limits: Option<LimitsConfig>,
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
            .field("tenant_id", &self.tenant_id)
            .field("project_id", &self.project_id)
            .field("user_id", &self.user_id)
            .field("tenant_budget", &self.tenant_budget)
            .field("project_budget", &self.project_budget)
            .field("user_budget", &self.user_budget)
            .field("tenant_limits", &self.tenant_limits)
            .field("project_limits", &self.project_limits)
            .field("user_limits", &self.user_limits)
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
            tenant_id: None,
            project_id: None,
            user_id: None,
            tenant_budget: None,
            project_budget: None,
            user_budget: None,
            tenant_limits: None,
            project_limits: None,
            user_limits: None,
            limits: LimitsConfig::default(),
            budget: BudgetConfig::default(),
            cache: CacheConfig::default(),
            guardrails: GuardrailsConfig::default(),
            passthrough: PassthroughConfig::default(),
            route: None,
        }
    }

    pub fn resolve_env(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        self.token = expand_env_placeholders(&self.token, env)?;
        Ok(())
    }

    pub async fn resolve_secrets(&mut self, env: &Env) -> Result<(), super::GatewayError> {
        resolve_secret_in_string(&mut self.token, env, "virtual_keys[].token").await?;
        Ok(())
    }

    pub(crate) fn token_lookup_key(&self) -> Option<String> {
        normalize_virtual_key_token_key(&self.token)
    }

    pub(crate) fn matches_token(&self, presented: &str) -> bool {
        let Some(expected) = self.token_lookup_key() else {
            return false;
        };
        normalize_virtual_key_token_key(presented).is_some_and(|actual| actual == expected)
    }

    pub(crate) fn sanitized_for_persistence(&self) -> Self {
        let mut sanitized = self.clone();
        sanitized.token = persisted_virtual_key_token(&self.token);
        sanitized
    }
}

pub(crate) fn persisted_virtual_key_token(token: &str) -> String {
    match normalize_virtual_key_token_key(token) {
        Some(hash) => format!("{VIRTUAL_KEY_TOKEN_HASH_PREFIX}{hash}"),
        None => token.to_string(),
    }
}

pub(crate) fn normalize_virtual_key_token_key(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(hash) = trimmed.strip_prefix(VIRTUAL_KEY_TOKEN_HASH_PREFIX) {
        let hash = hash.trim();
        if hash.is_empty() {
            return None;
        }
        return Some(hash.to_string());
    }

    Some(hash_sha256(trimmed.as_bytes()).to_string())
}

pub(crate) fn validate_virtual_key_configs(
    keys: &[VirtualKeyConfig],
) -> Result<(), super::GatewayError> {
    let mut seen_ids = std::collections::HashMap::<&str, usize>::new();
    let mut seen_tokens = std::collections::HashMap::<String, usize>::new();

    for (idx, key) in keys.iter().enumerate() {
        let id = key.id.trim();
        if id.is_empty() {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!("virtual_keys[{idx}].id cannot be empty"),
            });
        }
        if let Some(first_idx) = seen_ids.insert(id, idx) {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!(
                    "duplicate virtual key id `{id}` (first at index {first_idx}, duplicate at index {idx})"
                ),
            });
        }

        let Some(token_key) = normalize_virtual_key_token_key(&key.token) else {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!("virtual_keys[{idx}].token cannot be empty"),
            });
        };
        if let Some(first_idx) = seen_tokens.insert(token_key, idx) {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!(
                    "duplicate virtual key token (first at index {first_idx}, duplicate at index {idx})"
                ),
            });
        }
    }

    Ok(())
}

pub(crate) fn validate_virtual_key_routes(
    keys: &[VirtualKeyConfig],
    backend_names: &HashSet<String>,
) -> Result<(), super::GatewayError> {
    for (idx, key) in keys.iter().enumerate() {
        let Some(route) = key.route.as_deref() else {
            continue;
        };
        let route = route.trim();
        if route.is_empty() {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!("virtual_keys[{idx}].route cannot be empty"),
            });
        }
        if !backend_names.contains(route) {
            return Err(super::GatewayError::InvalidRequest {
                reason: format!("virtual_keys[{idx}].route references unknown backend: {route}"),
            });
        }
    }

    Ok(())
}

pub(crate) fn validate_guardrails_config(
    keys: &[VirtualKeyConfig],
    router: &RouterConfig,
) -> Result<(), super::GatewayError> {
    for (idx, key) in keys.iter().enumerate() {
        key.guardrails
            .validate()
            .map_err(|err| super::GatewayError::InvalidRequest {
                reason: format!("virtual_keys[{idx}].guardrails invalid: {err}"),
            })?;
    }

    for (idx, rule) in router.rules.iter().enumerate() {
        let Some(guardrails) = rule.guardrails.as_ref() else {
            continue;
        };
        guardrails
            .validate()
            .map_err(|err| super::GatewayError::InvalidRequest {
                reason: format!("router.rules[{idx}].guardrails invalid: {err}"),
            })?;
    }

    Ok(())
}

pub(crate) fn validate_router_against_backends(
    router: &RouterConfig,
    backend_names: &HashSet<String>,
) -> Result<(), super::GatewayError> {
    let mut unknown_refs: Vec<String> = Vec::new();
    let mut invalid_fields: Vec<String> = Vec::new();

    for (idx, backend) in router.default_backends.iter().enumerate() {
        let name = backend.backend.trim();
        if name.is_empty() {
            invalid_fields.push(format!("router.default_backends[{idx}].backend"));
            continue;
        }
        if !backend_names.contains(name) {
            unknown_refs.push(name.to_string());
        }
    }

    for (rule_idx, rule) in router.rules.iter().enumerate() {
        let model_prefix = rule.model_prefix.trim();
        if model_prefix.is_empty() {
            invalid_fields.push(format!("router.rules[{rule_idx}].model_prefix"));
        }

        let mut has_backend = false;
        let legacy_backend = rule.backend.trim();
        if !legacy_backend.is_empty() {
            has_backend = true;
            if !backend_names.contains(legacy_backend) {
                unknown_refs.push(legacy_backend.to_string());
            }
        }

        for (backend_idx, backend) in rule.backends.iter().enumerate() {
            let name = backend.backend.trim();
            if name.is_empty() {
                invalid_fields.push(format!(
                    "router.rules[{rule_idx}].backends[{backend_idx}].backend"
                ));
                continue;
            }
            has_backend = true;
            if !backend_names.contains(name) {
                unknown_refs.push(name.to_string());
            }
        }

        if !has_backend {
            invalid_fields.push(format!(
                "router.rules[{rule_idx}] requires `backend` or non-empty `backends[]`"
            ));
        }
    }

    if !invalid_fields.is_empty() {
        return Err(super::GatewayError::InvalidRequest {
            reason: format!(
                "invalid router config fields: {}",
                invalid_fields.join(", ")
            ),
        });
    }

    if !unknown_refs.is_empty() {
        unknown_refs.sort();
        unknown_refs.dedup();
        return Err(super::GatewayError::InvalidRequest {
            reason: format!(
                "router references unknown backends: {}",
                unknown_refs.join(", ")
            ),
        });
    }

    Ok(())
}

async fn resolve_secret_in_string(
    value: &mut String,
    env: &Env,
    label: &str,
) -> Result<(), super::GatewayError> {
    let trimmed = value.trim();
    if !trimmed.starts_with("secret://") {
        return Ok(());
    }

    let resolved = resolve_secret(trimmed, env)
        .await
        .map(|secret| secret.into_owned())
        .map_err(|err| super::GatewayError::InvalidRequest {
            reason: format!("failed to resolve secret for {label}: {err}"),
        })?;
    *value = resolved;
    Ok(())
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
            max_in_flight: None,
            timeout_seconds: None,
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
    fn backend_config_resolves_env_placeholders_in_provider_config() {
        let env = Env {
            dotenv: BTreeMap::from([
                (
                    "BASE_URL".to_string(),
                    "https://api.example.com/v1".to_string(),
                ),
                ("DEFAULT_MODEL".to_string(), "gpt-4o-mini".to_string()),
                ("API_VERSION".to_string(), "2024-01-01".to_string()),
                ("AUTH_TOKEN".to_string(), "sk-test".to_string()),
                ("MODEL_PREFIX".to_string(), "gpt".to_string()),
            ]),
        };

        let provider_config = ProviderConfig {
            provider: None,
            enabled_capabilities: Vec::new(),
            base_url: Some("${BASE_URL}".to_string()),
            default_model: Some("${DEFAULT_MODEL}".to_string()),
            model_whitelist: vec!["${MODEL_PREFIX}-*".to_string()],
            http_headers: BTreeMap::from([(
                "authorization".to_string(),
                "Bearer ${AUTH_TOKEN}".to_string(),
            )]),
            http_query_params: BTreeMap::from([(
                "api-version".to_string(),
                "${API_VERSION}".to_string(),
            )]),
            auth: None,
            capabilities: None,
            upstream_api: None,
            normalize_to: None,
            normalize_endpoint: None,
            openai_compatible: None,
        };

        let mut backend = BackendConfig {
            name: "translation".to_string(),
            base_url: String::new(),
            max_in_flight: None,
            timeout_seconds: None,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            provider: Some("openai-compatible".to_string()),
            provider_config: Some(provider_config),
            model_map: BTreeMap::new(),
        };

        backend.resolve_env(&env).expect("resolve");
        let provider_config = backend.provider_config.expect("provider config");
        assert_eq!(
            provider_config.base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            provider_config.default_model.as_deref(),
            Some("gpt-4o-mini")
        );
        assert_eq!(provider_config.model_whitelist, vec!["gpt-*".to_string()]);
        assert_eq!(
            provider_config
                .http_headers
                .get("authorization")
                .map(|s| s.as_str()),
            Some("Bearer sk-test")
        );
        assert_eq!(
            provider_config
                .http_query_params
                .get("api-version")
                .map(|s| s.as_str()),
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
            max_in_flight: None,
            timeout_seconds: None,
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

    #[test]
    fn backend_config_resolves_litellm_os_environ_strings() {
        let env = Env {
            dotenv: BTreeMap::from([("BASE_URL".to_string(), "https://example.com".to_string())]),
        };

        let mut backend = BackendConfig {
            name: "primary".to_string(),
            base_url: "os.environ/BASE_URL".to_string(),
            max_in_flight: None,
            timeout_seconds: None,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            provider: None,
            provider_config: None,
            model_map: BTreeMap::new(),
        };

        backend.resolve_env(&env).expect("resolve");
        assert_eq!(backend.base_url, "https://example.com");
    }

    #[test]
    fn virtual_key_config_resolves_env_placeholder_in_token() {
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_VK_TOKEN".to_string(), "vk-1".to_string())]),
        };

        let mut key = VirtualKeyConfig::new("key-1", "${DITTO_TEST_VK_TOKEN}");
        key.resolve_env(&env).expect("resolve");
        assert_eq!(key.token, "vk-1");
    }

    #[test]
    fn virtual_key_config_resolves_litellm_os_environ_token() {
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_VK_TOKEN".to_string(), "vk-1".to_string())]),
        };

        let mut key = VirtualKeyConfig::new("key-1", "os.environ/DITTO_TEST_VK_TOKEN");
        key.resolve_env(&env).expect("resolve");
        assert_eq!(key.token, "vk-1");
    }

    #[tokio::test]
    async fn gateway_config_resolves_secret_specs() {
        let env = Env {
            dotenv: BTreeMap::from([("REAL_TOKEN".to_string(), "vk-1".to_string())]),
        };

        let mut config = GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![VirtualKeyConfig::new("key-1", "secret://env/REAL_TOKEN")],
            router: RouterConfig {
                default_backends: Vec::new(),
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        config.resolve_secrets(&env).await.expect("resolve secrets");
        assert_eq!(config.virtual_keys[0].token, "vk-1");
    }

    #[test]
    fn gateway_config_validate_rejects_unknown_virtual_key_route() {
        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.route = Some("missing-backend".to_string());

        let config = GatewayConfig {
            backends: vec![BackendConfig {
                name: "primary".to_string(),
                base_url: "https://example.com".to_string(),
                max_in_flight: None,
                timeout_seconds: None,
                headers: BTreeMap::new(),
                query_params: BTreeMap::new(),
                provider: None,
                provider_config: None,
                model_map: BTreeMap::new(),
            }],
            virtual_keys: vec![key],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let err = config.validate().expect_err("unknown route should fail");
        assert!(
            err.to_string()
                .contains("virtual_keys[0].route references unknown backend")
        );
    }

    #[test]
    fn gateway_config_validate_rejects_invalid_guardrails() {
        let mut key = VirtualKeyConfig::new("key-1", "vk-1");
        key.guardrails.banned_regexes = vec!["(".to_string()];

        let config = GatewayConfig {
            backends: vec![BackendConfig {
                name: "primary".to_string(),
                base_url: "https://example.com".to_string(),
                max_in_flight: None,
                timeout_seconds: None,
                headers: BTreeMap::new(),
                query_params: BTreeMap::new(),
                provider: None,
                provider_config: None,
                model_map: BTreeMap::new(),
            }],
            virtual_keys: vec![key],
            router: RouterConfig {
                default_backends: vec![RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let err = config
            .validate()
            .expect_err("invalid guardrails should fail");
        assert!(
            err.to_string()
                .contains("virtual_keys[0].guardrails invalid")
        );
    }
}
