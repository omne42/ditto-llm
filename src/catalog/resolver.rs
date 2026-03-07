use crate::profile::ProviderConfig;
use crate::{DittoError, Result};

use super::{
    CatalogRegistry, HttpMethod, InvocationHints, OperationKind, ResolvedInvocation, TransportKind,
};

#[derive(Debug, Clone, Copy)]
pub struct RuntimeRouteRequest<'a> {
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub operation: OperationKind,
    pub provider_config: Option<&'a ProviderConfig>,
    pub hints: InvocationHints,
}

impl<'a> RuntimeRouteRequest<'a> {
    pub fn new(provider: &'a str, model: Option<&'a str>, operation: OperationKind) -> Self {
        Self {
            provider,
            model,
            operation,
            provider_config: None,
            hints: InvocationHints::default(),
        }
    }

    pub fn with_provider_config(mut self, provider_config: &'a ProviderConfig) -> Self {
        self.provider_config = Some(provider_config);
        self
    }

    pub fn with_hints(mut self, hints: InvocationHints) -> Self {
        self.hints = hints;
        self
    }
}

impl Default for RuntimeRouteRequest<'_> {
    fn default() -> Self {
        Self::new("", None, OperationKind::CHAT_COMPLETION)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRoute {
    pub invocation: ResolvedInvocation,
    pub base_url: String,
    pub url: String,
    pub query_params: Vec<(String, String)>,
}

impl RuntimeRoute {
    pub fn transport(&self) -> TransportKind {
        self.invocation.endpoint.transport
    }

    pub fn http_method(&self) -> Option<HttpMethod> {
        self.invocation.endpoint.http_method
    }

    pub fn path(&self) -> &str {
        self.invocation.endpoint.path.as_str()
    }
}

impl CatalogRegistry {
    pub fn resolve_runtime_route(&self, request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
        let provider = request.provider.trim();
        if provider.is_empty() {
            return Err(DittoError::InvalidResponse(
                "runtime route provider must be non-empty".to_string(),
            ));
        }

        let plugin = self.plugin(provider).ok_or_else(|| {
            DittoError::InvalidResponse(format!("catalog provider not found: {provider}"))
        })?;
        let model = resolve_runtime_model(request.model, request.provider_config)?;
        let invocation = self
            .resolve_with_hints(provider, model.as_str(), request.operation, request.hints)
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "catalog route not found for provider={provider} model={} operation={}",
                    model, request.operation
                ))
            })?;

        let base_url = resolve_runtime_base_url(
            invocation.endpoint.base_url_override.as_deref(),
            request.provider_config,
            plugin.default_base_url,
        )?;
        let query_params =
            merge_runtime_query_params(&invocation.endpoint.query_params, request.provider_config);
        let joined = join_base_url(base_url.as_str(), invocation.endpoint.path.as_str());
        let url = append_query_params(joined, &query_params);

        Ok(RuntimeRoute {
            invocation,
            base_url,
            url,
            query_params,
        })
    }
}

fn resolve_runtime_model(
    explicit_model: Option<&str>,
    provider_config: Option<&ProviderConfig>,
) -> Result<String> {
    if let Some(model) = explicit_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok(model.to_string());
    }

    if let Some(model) = provider_config
        .and_then(|config| config.default_model.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok(model.to_string());
    }

    Err(DittoError::InvalidResponse(
        "runtime route model is not set (provide model or provider_config.default_model)"
            .to_string(),
    ))
}

fn resolve_runtime_base_url(
    endpoint_override: Option<&str>,
    provider_config: Option<&ProviderConfig>,
    plugin_default: Option<&str>,
) -> Result<String> {
    endpoint_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            provider_config
                .and_then(|config| config.base_url.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            plugin_default
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(|value| value.to_string())
        .ok_or_else(|| {
            DittoError::InvalidResponse(
                "runtime route base_url is not set (missing endpoint override, provider_config.base_url, and plugin default_base_url)".to_string(),
            )
        })
}

fn merge_runtime_query_params(
    endpoint_query_params: &[(String, String)],
    provider_config: Option<&ProviderConfig>,
) -> Vec<(String, String)> {
    let mut out = endpoint_query_params.to_vec();
    if let Some(provider_config) = provider_config {
        for (name, value) in &provider_config.http_query_params {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            out.push((name.to_string(), value.clone()));
        }
    }
    out
}

fn join_base_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path_no_leading_slash = path.strip_prefix('/').unwrap_or(path);

    if base.ends_with("/v1") {
        if path_no_leading_slash == "v1" {
            return base.to_string();
        }
        if let Some(rest) = path_no_leading_slash.strip_prefix("v1/") {
            let mut out = String::with_capacity(base.len() + 1 + rest.len());
            out.push_str(base);
            out.push('/');
            out.push_str(rest);
            return out;
        }
    }

    if path.starts_with('/') {
        let mut out = String::with_capacity(base.len() + path.len());
        out.push_str(base);
        out.push_str(path);
        out
    } else {
        let mut out = String::with_capacity(base.len() + 1 + path.len());
        out.push_str(base);
        out.push('/');
        out.push_str(path);
        out
    }
}

fn append_query_params(base_url: String, query_params: &[(String, String)]) -> String {
    if query_params.is_empty() {
        return base_url;
    }

    let mut out = base_url;
    out.push(if out.contains('?') { '&' } else { '?' });
    for (idx, (name, value)) in query_params.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        out.push_str(name);
        out.push('=');
        out.push_str(value);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::catalog::{
        ApiSurfaceId, AuthMethodKind, EndpointQueryParam, EndpointTemplate, ModelBinding,
        ModelSelector, ProtocolQuirks, ProviderClass, ProviderPluginDescriptor, TransportKind,
        VerificationStatus, WireProtocol,
    };

    #[test]
    fn runtime_route_uses_plugin_default_base_url() {
        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "openai",
                Some("gpt-4.1"),
                OperationKind::CHAT_COMPLETION,
            ))
            .expect("runtime route should resolve");

        assert_eq!(route.base_url, "https://api.openai.com/v1");
        assert_eq!(route.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(route.transport(), TransportKind::Http);
    }

    #[test]
    fn runtime_route_uses_provider_config_default_model_and_query_params() {
        let mut provider_config = ProviderConfig::default();
        provider_config.base_url = Some("http://localhost:8080/v1".to_string());
        provider_config.default_model = Some("gpt-4.1".to_string());
        provider_config
            .http_query_params
            .insert("api-version".to_string(), "2025-03-01".to_string());

        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new("openai", None, OperationKind::CHAT_COMPLETION)
                    .with_provider_config(&provider_config),
            )
            .expect("runtime route should resolve with provider config default model");

        assert_eq!(route.base_url, "http://localhost:8080/v1");
        assert_eq!(
            route.url,
            "http://localhost:8080/v1/chat/completions?api-version=2025-03-01"
        );
        assert_eq!(
            route.query_params,
            vec![("api-version".to_string(), "2025-03-01".to_string())]
        );
    }

    #[test]
    fn runtime_route_prefers_endpoint_base_url_override() {
        const BINDINGS: &[ModelBinding] = &[ModelBinding {
            operation: OperationKind::REALTIME_SESSION,
            selector: ModelSelector::Exact(&["glm-realtime"]),
            surface: ApiSurfaceId::OPENAI_REALTIME,
            wire_protocol: WireProtocol::ZHIPU_NATIVE,
            endpoint: EndpointTemplate {
                transport: TransportKind::WebSocket,
                http_method: None,
                base_url_override: Some("wss://open.bigmodel.cn"),
                path_template: "/api/paas/v4/realtime",
                query_params: &[EndpointQueryParam {
                    name: "model",
                    value_template: "{model}",
                }],
            },
            quirks: Some(ProtocolQuirks {
                require_model_prefix: true,
                supports_system_role: true,
                force_stream_options: false,
            }),
            streaming: Some(true),
            async_job: None,
            verification: VerificationStatus::Explicit,
            evidence: &[],
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "zhipu-like",
            display_name: "Zhipu Like",
            class: ProviderClass::Custom,
            default_base_url: Some("https://ignored.example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
        };
        let registry = CatalogRegistry::new(&[PLUGIN]);

        let mut provider_config = ProviderConfig::default();
        provider_config.base_url = Some("https://also-ignored.example.com/v1".to_string());

        let route = registry
            .resolve_runtime_route(
                RuntimeRouteRequest::new(
                    "zhipu-like",
                    Some("glm-realtime"),
                    OperationKind::REALTIME_SESSION,
                )
                .with_provider_config(&provider_config),
            )
            .expect("endpoint override should win");

        assert_eq!(route.base_url, "wss://open.bigmodel.cn");
        assert_eq!(
            route.url,
            "wss://open.bigmodel.cn/api/paas/v4/realtime?model=glm-realtime"
        );
        assert_eq!(route.invocation.quirks.require_model_prefix, true);
    }

    #[test]
    fn runtime_route_respects_v1_join_ergonomics() {
        assert_eq!(
            join_base_url("http://localhost:8080/v1", "/v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            join_base_url("http://localhost:8080/v1", "v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            join_base_url("http://localhost:8080/v1", "/v1"),
            "http://localhost:8080/v1"
        );
    }

    #[test]
    fn runtime_route_requires_model_and_base_url() {
        let err = crate::catalog::builtin_registry()
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "openai",
                None,
                OperationKind::CHAT_COMPLETION,
            ))
            .expect_err("missing model should fail");
        assert!(matches!(err, DittoError::InvalidResponse(_)));

        const BINDINGS: &[ModelBinding] = &[ModelBinding {
            operation: OperationKind::CHAT_COMPLETION,
            selector: ModelSelector::Any,
            surface: ApiSurfaceId::OPENAI_CHAT_COMPLETIONS,
            wire_protocol: WireProtocol::OPENAI_CHAT_COMPLETIONS,
            endpoint: EndpointTemplate {
                transport: TransportKind::Http,
                http_method: Some(HttpMethod::Post),
                base_url_override: None,
                path_template: "/v1/chat/completions",
                query_params: &[],
            },
            quirks: None,
            streaming: None,
            async_job: None,
            verification: VerificationStatus::Explicit,
            evidence: &[],
        }];
        const PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "no-base-url",
            display_name: "No Base URL",
            class: ProviderClass::Custom,
            default_base_url: None,
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
        };
        let registry = CatalogRegistry::new(&[PLUGIN]);
        let err = registry
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "no-base-url",
                Some("anything"),
                OperationKind::CHAT_COMPLETION,
            ))
            .expect_err("missing base url should fail");
        assert!(matches!(err, DittoError::InvalidResponse(_)));
    }

    #[test]
    fn merge_runtime_query_params_ignores_blank_names() {
        let mut provider_config = ProviderConfig::default();
        provider_config
            .http_query_params
            .insert(" ".to_string(), "ignored".to_string());
        provider_config
            .http_query_params
            .insert("api-version".to_string(), "1".to_string());

        let merged = merge_runtime_query_params(
            &[("alt".to_string(), "sse".to_string())],
            Some(&provider_config),
        );
        assert_eq!(
            merged,
            vec![
                ("alt".to_string(), "sse".to_string()),
                ("api-version".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn append_query_params_appends_with_existing_separator() {
        assert_eq!(
            append_query_params(
                "https://example.com/path".to_string(),
                &[
                    ("a".to_string(), "1".to_string()),
                    ("b".to_string(), "2".to_string())
                ]
            ),
            "https://example.com/path?a=1&b=2"
        );
        assert_eq!(
            append_query_params(
                "https://example.com/path?x=0".to_string(),
                &[("a".to_string(), "1".to_string())]
            ),
            "https://example.com/path?x=0&a=1"
        );
    }

    #[test]
    fn runtime_route_keeps_invocation_metadata() {
        let mut provider_config = ProviderConfig::default();
        provider_config.base_url = Some("https://example.com/v1".to_string());
        provider_config.default_model = Some("gpt-4.1".to_string());
        provider_config.http_query_params = BTreeMap::new();

        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new("openai", None, OperationKind::CHAT_COMPLETION)
                    .with_provider_config(&provider_config),
            )
            .expect("route should resolve");

        assert_eq!(route.invocation.provider, "openai");
        assert_eq!(route.invocation.model, "gpt-4.1");
        assert_eq!(
            route.invocation.surface,
            ApiSurfaceId::OPENAI_CHAT_COMPLETIONS
        );
    }
}
