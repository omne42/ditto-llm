use std::collections::BTreeMap;

use serde::Deserialize;

use super::{BackendConfig, GatewayConfig, GatewayError, RouteBackend, RouteRule, RouterConfig};

#[derive(Debug, Deserialize)]
pub struct LitellmProxyConfig {
    #[serde(default)]
    pub model_list: Vec<LitellmModelEntry>,
    #[serde(default)]
    pub general_settings: Option<LitellmGeneralSettings>,
}

#[derive(Debug, Deserialize)]
pub struct LitellmGeneralSettings {
    #[serde(default)]
    pub master_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LitellmModelEntry {
    pub model_name: String,
    pub litellm_params: LitellmParams,
}

#[derive(Debug, Deserialize)]
pub struct LitellmParams {
    pub model: String,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_version: Option<serde_yaml::Value>,
    #[serde(default)]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub stream_timeout: Option<f64>,
}

impl LitellmProxyConfig {
    pub fn try_into_gateway_config(self) -> Result<GatewayConfig, GatewayError> {
        if self.model_list.is_empty() {
            return Err(GatewayError::InvalidRequest {
                reason: "litellm config missing model_list".to_string(),
            });
        }

        let mut model_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut backends = Vec::<BackendConfig>::new();
        for (idx, entry) in self.model_list.into_iter().enumerate() {
            let model_name = entry.model_name.trim().to_string();
            if model_name.is_empty() {
                return Err(GatewayError::InvalidRequest {
                    reason: "litellm config contains empty model_name".to_string(),
                });
            }

            let backend_name = format!("litellm_{idx}_{}", sanitize_backend_name(&model_name));
            let backend =
                backend_from_litellm_entry(&backend_name, &model_name, entry.litellm_params)?;

            model_groups
                .entry(model_name)
                .or_default()
                .push(backend_name.clone());
            backends.push(backend);
        }

        let master_key = self
            .general_settings
            .and_then(|settings| settings.master_key)
            .and_then(|value| {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
        let virtual_keys = match master_key {
            Some(master_key) => vec![super::VirtualKeyConfig::new(
                "litellm_master_key",
                master_key,
            )],
            None => Vec::new(),
        };

        let (default_backend, default_backends, mut rules) =
            build_router_from_model_groups(&model_groups)?;

        rules.sort_by(|a, b| match (a.exact, b.exact) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.model_prefix.len().cmp(&a.model_prefix.len()),
        });

        Ok(GatewayConfig {
            backends,
            virtual_keys,
            router: RouterConfig {
                default_backend,
                default_backends,
                rules,
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
        })
    }
}

fn build_router_from_model_groups(
    groups: &BTreeMap<String, Vec<String>>,
) -> Result<(String, Vec<RouteBackend>, Vec<RouteRule>), GatewayError> {
    let mut default_backend = String::new();
    let mut default_backends: Vec<RouteBackend> = Vec::new();
    let mut rules: Vec<RouteRule> = Vec::new();

    let wildcard_backends = groups.get("*").cloned().unwrap_or_default();
    if !wildcard_backends.is_empty() {
        if wildcard_backends.len() == 1 {
            default_backend = wildcard_backends[0].clone();
        } else {
            default_backends = wildcard_backends
                .iter()
                .map(|name| RouteBackend {
                    backend: name.clone(),
                    weight: 1,
                })
                .collect();
        }
    }

    for (model_name, backends) in groups {
        if model_name == "*" {
            continue;
        }

        let exact = !model_name.contains('*');
        if backends.is_empty() {
            continue;
        }
        if backends.len() == 1 {
            rules.push(RouteRule {
                model_prefix: model_name.clone(),
                exact,
                backend: backends[0].clone(),
                backends: Vec::new(),
                guardrails: None,
            });
            continue;
        }

        rules.push(RouteRule {
            model_prefix: model_name.clone(),
            exact,
            backend: String::new(),
            backends: backends
                .iter()
                .map(|name| RouteBackend {
                    backend: name.clone(),
                    weight: 1,
                })
                .collect(),
            guardrails: None,
        });
    }

    if default_backend.trim().is_empty() && default_backends.is_empty() {
        default_backend = groups
            .values()
            .flat_map(|values| values.iter())
            .next()
            .cloned()
            .ok_or_else(|| GatewayError::InvalidRequest {
                reason: "litellm config produced no backends".to_string(),
            })?;
    }

    Ok((default_backend, default_backends, rules))
}

fn backend_from_litellm_entry(
    backend_name: &str,
    model_name: &str,
    params: LitellmParams,
) -> Result<BackendConfig, GatewayError> {
    let base_url = params
        .api_base
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    let mut headers = BTreeMap::<String, String>::new();
    if let Some(api_key) = params
        .api_key
        .as_deref()
        .and_then(normalize_litellm_key_ref)
    {
        headers.insert("authorization".to_string(), format!("Bearer {api_key}"));
    } else {
        headers.insert(
            "authorization".to_string(),
            "Bearer ${OPENAI_API_KEY}".to_string(),
        );
    }

    let mut query_params = BTreeMap::<String, String>::new();
    if let Some(api_version) = params.api_version.as_ref().and_then(yaml_scalar_to_string) {
        if !api_version.trim().is_empty() {
            query_params.insert("api-version".to_string(), api_version);
        }
    }

    let mut model_map = BTreeMap::<String, String>::new();
    if !model_name.contains('*') && !params.model.contains('*') {
        let mapped_model = strip_provider_prefix(&params.model);
        if !mapped_model.trim().is_empty() && mapped_model.trim() != model_name.trim() {
            model_map.insert(model_name.to_string(), mapped_model);
        }
    }

    let timeout_seconds = params
        .timeout
        .or(params.stream_timeout)
        .map(|secs| secs.ceil() as u64)
        .filter(|secs| *secs > 0);

    Ok(BackendConfig {
        name: backend_name.to_string(),
        base_url,
        max_in_flight: None,
        timeout_seconds,
        headers,
        query_params,
        provider: None,
        provider_config: None,
        model_map,
    })
}

fn yaml_scalar_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn strip_provider_prefix(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        return String::new();
    }
    if let Some((_provider, rest)) = model.split_once('/') {
        return rest.trim().to_string();
    }
    model.to_string()
}

fn normalize_litellm_key_ref(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(name) = value.strip_prefix("os.environ/") {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        return Some(format!("${{{name}}}"));
    }

    if let Some(rest) = value.strip_prefix("os.environ[") {
        let inner = rest.trim_end_matches(']').trim();
        let inner = inner.trim_matches('\'').trim_matches('"').trim();
        if inner.is_empty() {
            return None;
        }
        return Some(format!("${{{inner}}}"));
    }

    Some(value.to_string())
}

fn sanitize_backend_name(model_name: &str) -> String {
    let mut out = String::with_capacity(model_name.len());
    for c in model_name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_basic_proxy_config_yaml() {
        let raw = r#"
model_list:
  - model_name: "*"
    litellm_params:
      model: "*"

general_settings:
  master_key: sk-1234
"#;

        let parsed: LitellmProxyConfig = serde_yaml::from_str(raw).expect("parse litellm config");
        let config = parsed.try_into_gateway_config().expect("convert");
        assert_eq!(config.virtual_keys.len(), 1);
        assert_eq!(config.virtual_keys[0].token, "sk-1234");
        assert!(!config.backends.is_empty());
        assert!(
            !config.router.default_backend.trim().is_empty()
                || !config.router.default_backends.is_empty()
        );
    }

    #[test]
    fn imports_exact_model_with_model_map() {
        let raw = r#"
model_list:
  - model_name: gpt-4
    litellm_params:
      model: openai/gpt-4.1-mini
      api_key: os.environ/OPENAI_API_KEY
"#;

        let parsed: LitellmProxyConfig = serde_yaml::from_str(raw).expect("parse litellm config");
        let config = parsed.try_into_gateway_config().expect("convert");
        assert_eq!(config.router.rules.len(), 1);
        let rule = &config.router.rules[0];
        assert!(rule.exact);
        let backend = config
            .backends
            .iter()
            .find(|b| b.name == rule.backend)
            .expect("backend");
        assert_eq!(
            backend.headers.get("authorization").map(|s| s.as_str()),
            Some("Bearer ${OPENAI_API_KEY}")
        );
        assert_eq!(
            backend.model_map.get("gpt-4").map(|s| s.as_str()),
            Some("gpt-4.1-mini")
        );
    }
}
