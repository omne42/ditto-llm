use crate::contracts::{
    CapabilityKind, InvocationHints, OperationKind, ProviderId, ResolvedInvocation,
    RuntimeProviderApi, RuntimeProviderHints, RuntimeRoute, RuntimeRouteRequest, TransportKind,
};
use crate::{ProviderResolutionError, Result};

use super::{CatalogRegistry, ProviderPluginDescriptor};
impl CatalogRegistry {
    pub fn plugin_for_runtime_request(
        &self,
        provider_name_hint: &str,
        provider_hints: RuntimeProviderHints<'_>,
    ) -> Option<&'static ProviderPluginDescriptor> {
        let provider = provider_name_hint.trim();
        if let Some(plugin) = (!provider.is_empty())
            .then(|| {
                self.plugin_by_id(ProviderId::new(provider))
                    .or_else(|| self.plugin_by_hint(provider))
            })
            .flatten()
        {
            return Some(plugin);
        }

        if let Some(configured_provider) = configured_provider_hint(provider_hints) {
            if let Some(plugin) = self
                .plugin_by_id(ProviderId::new(configured_provider))
                .or_else(|| self.plugin_by_hint(configured_provider))
            {
                return Some(plugin);
            }
        }

        fallback_runtime_plugin(self, provider_hints)
    }

    pub fn resolve_runtime_route(&self, request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
        let provider = request.provider_id();
        let provider = ProviderId::new(provider.as_str().trim());
        let provider_hint =
            effective_runtime_provider_hint(provider.as_str(), request.provider_hints);
        if provider_hint.is_empty() {
            return Err(ProviderResolutionError::RuntimeRouteProviderMissing.into());
        }

        let plugin = self
            .plugin_for_runtime_request(provider.as_str(), request.provider_hints)
            .ok_or_else(|| ProviderResolutionError::CatalogProviderNotFound {
                provider: provider_hint.to_string(),
            })?;
        let model = resolve_runtime_model(request.model, request.provider_hints)?;
        if let Some(capability) = request.required_capability {
            validate_configured_enabled_capabilities(
                request.provider_hints,
                plugin.id,
                capability,
            )?;
            let resolution = plugin.capability_resolution(Some(model.as_str()));
            if !resolution.effective_supports(capability) {
                return Err(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                    provider: plugin.id.to_string(),
                    model,
                    capability: capability.to_string(),
                }
                .into());
            }
        }
        let hints = normalize_runtime_hints(request.operation, request.hints);
        let invocation = self
            .resolve_with_hints_for_provider(
                plugin.provider_id(),
                model.as_str(),
                request.operation,
                hints,
            )
            .ok_or_else(|| ProviderResolutionError::CatalogRouteNotFound {
                provider: plugin.id.to_string(),
                model: model.clone(),
                operation: request.operation.to_string(),
            })?;
        let invocation =
            normalize_runtime_invocation(provider_hint, request.provider_hints, invocation);

        let base_url = adapt_runtime_base_url_for_transport(
            resolve_runtime_base_url(
                provider_hint,
                invocation.endpoint.base_url_override.as_deref(),
                request.provider_hints,
                plugin.default_base_url,
            )?,
            invocation.endpoint.transport,
        );
        let query_params =
            merge_runtime_query_params(&invocation.endpoint.query_params, request.provider_hints);
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

fn configured_provider_hint(provider_hints: RuntimeProviderHints<'_>) -> Option<&str> {
    provider_hints
        .configured_provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn effective_runtime_provider_hint<'a>(
    provider_name_hint: &'a str,
    provider_hints: RuntimeProviderHints<'a>,
) -> &'a str {
    let provider = provider_name_hint.trim();
    if !provider.is_empty() {
        return provider;
    }
    configured_provider_hint(provider_hints).unwrap_or("")
}

fn validate_configured_enabled_capabilities(
    provider_hints: RuntimeProviderHints<'_>,
    provider: &str,
    required_capability: CapabilityKind,
) -> Result<()> {
    if provider_hints.enabled_capabilities.is_empty() {
        return Ok(());
    }

    let mut configured_supports_required = false;
    for capability in provider_hints.enabled_capabilities {
        let Some(parsed) = CapabilityKind::parse_config_token(capability) else {
            return Err(ProviderResolutionError::ConfiguredCapabilityUnknown {
                capability: capability.trim().to_string(),
            }
            .into());
        };
        if parsed == required_capability {
            configured_supports_required = true;
        }
    }

    if configured_supports_required {
        return Ok(());
    }

    Err(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
        provider: provider.to_string(),
        model: provider_hints
            .default_model
            .map(str::to_string)
            .unwrap_or_else(|| "<unspecified>".to_string()),
        capability: required_capability.to_string(),
    }
    .into())
}

fn normalize_runtime_hints(
    operation: OperationKind,
    mut hints: InvocationHints,
) -> InvocationHints {
    if operation == OperationKind::REALTIME_SESSION && hints.streaming.is_none() {
        hints.streaming = Some(true);
    }
    hints
}

fn resolve_runtime_model(
    explicit_model: Option<&str>,
    provider_hints: RuntimeProviderHints<'_>,
) -> Result<String> {
    if let Some(model) = explicit_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok(model.to_string());
    }

    if let Some(model) = provider_hints
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok(model.to_string());
    }

    Err(ProviderResolutionError::RuntimeRouteModelMissing.into())
}

fn fallback_runtime_plugin(
    registry: &CatalogRegistry,
    provider_hints: RuntimeProviderHints<'_>,
) -> Option<&'static ProviderPluginDescriptor> {
    let upstream_api = provider_hints.upstream_api;
    match upstream_api {
        Some(RuntimeProviderApi::GeminiGenerateContent) => {
            registry.plugin_by_id(ProviderId::new("google"))
        }
        Some(RuntimeProviderApi::AnthropicMessages) => {
            registry.plugin_by_id(ProviderId::new("anthropic"))
        }
        Some(RuntimeProviderApi::OpenaiChatCompletions)
        | Some(RuntimeProviderApi::OpenaiResponses)
        | None => registry
            .plugin_by_id(ProviderId::new("openai-compatible"))
            .or_else(|| registry.plugin_by_id(ProviderId::new("openai"))),
    }
}

fn resolve_runtime_base_url(
    provider_name_hint: &str,
    endpoint_override: Option<&str>,
    provider_hints: RuntimeProviderHints<'_>,
    plugin_default: Option<&str>,
) -> Result<String> {
    endpoint_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            provider_hints
                .base_url
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            plugin_default
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(|value| value.to_string())
        .or_else(|| inferred_runtime_base_url(provider_name_hint, provider_hints))
        .ok_or_else(|| ProviderResolutionError::RuntimeRouteBaseUrlMissing.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAiPathStyle {
    KeepVersionPrefix,
    StripVersionPrefix,
}

#[derive(Debug, Clone, Copy)]
struct InferredRuntimeProviderHint {
    aliases: &'static [&'static str],
    base_url: &'static str,
    openai_path_style: OpenAiPathStyle,
}

const INFERRED_RUNTIME_PROVIDER_HINTS: &[InferredRuntimeProviderHint] = &[
    InferredRuntimeProviderHint {
        aliases: &["openrouter"],
        base_url: "https://openrouter.ai/api/v1",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["google", "gemini"],
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["anthropic", "claude"],
        base_url: "https://api.anthropic.com/v1",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["openai"],
        base_url: "https://api.openai.com/v1",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["deepseek"],
        base_url: "https://api.deepseek.com",
        openai_path_style: OpenAiPathStyle::StripVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["moonshot", "kimi"],
        base_url: "https://api.moonshot.cn/v1",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
    InferredRuntimeProviderHint {
        aliases: &["xai", "grok"],
        base_url: "https://api.x.ai/v1",
        openai_path_style: OpenAiPathStyle::KeepVersionPrefix,
    },
];

fn inferred_runtime_provider_hint(
    provider_name_hint: &str,
) -> Option<&'static InferredRuntimeProviderHint> {
    let provider_name = provider_name_hint.trim().to_ascii_lowercase();
    INFERRED_RUNTIME_PROVIDER_HINTS.iter().find(|hint| {
        hint.aliases
            .iter()
            .any(|alias| provider_name.contains(alias))
    })
}

fn normalize_runtime_invocation(
    provider_name_hint: &str,
    provider_hints: RuntimeProviderHints<'_>,
    mut invocation: ResolvedInvocation,
) -> ResolvedInvocation {
    let provider_name_hint = runtime_provider_inference_hint(provider_name_hint, provider_hints);
    if invocation.provider == "openai-compatible"
        && inferred_runtime_provider_hint(provider_name_hint)
            .map(|hint| hint.openai_path_style == OpenAiPathStyle::StripVersionPrefix)
            .unwrap_or(false)
    {
        if let Some(stripped) = invocation.endpoint.path.strip_prefix("/v1") {
            invocation.endpoint.path = if stripped.is_empty() {
                "/".to_string()
            } else {
                stripped.to_string()
            };
        }
    }
    invocation
}

fn runtime_provider_inference_hint<'a>(
    provider_name_hint: &'a str,
    provider_hints: RuntimeProviderHints<'a>,
) -> &'a str {
    let provider = provider_name_hint.trim();
    if !provider.is_empty() && inferred_runtime_provider_hint(provider).is_some() {
        return provider;
    }
    configured_provider_hint(provider_hints).unwrap_or(provider)
}

fn inferred_runtime_base_url(
    provider_name_hint: &str,
    provider_hints: RuntimeProviderHints<'_>,
) -> Option<String> {
    let provider_name_hint = runtime_provider_inference_hint(provider_name_hint, provider_hints);

    match provider_hints.upstream_api {
        Some(RuntimeProviderApi::GeminiGenerateContent) => {
            return Some("https://generativelanguage.googleapis.com/v1beta".to_string());
        }
        Some(RuntimeProviderApi::AnthropicMessages) => {
            return Some("https://api.anthropic.com/v1".to_string());
        }
        Some(RuntimeProviderApi::OpenaiChatCompletions)
        | Some(RuntimeProviderApi::OpenaiResponses)
        | None => {}
    }

    inferred_runtime_provider_hint(provider_name_hint).map(|hint| hint.base_url.to_string())
}

fn adapt_runtime_base_url_for_transport(base_url: String, transport: TransportKind) -> String {
    if transport == TransportKind::WebSocket {
        return crate::utils::http::to_websocket_base_url(&base_url);
    }
    base_url
}

fn merge_runtime_query_params(
    endpoint_query_params: &[(String, String)],
    provider_hints: RuntimeProviderHints<'_>,
) -> Vec<(String, String)> {
    let mut out = endpoint_query_params.to_vec();
    if let Some(query_params) = provider_hints.http_query_params {
        for (name, value) in query_params {
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
    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    use std::collections::BTreeMap;

    use super::*;
    use crate::catalog::{
        ApiSurfaceId, AuthMethodKind, EndpointQueryParam, EndpointTemplate, HttpMethod,
        ModelBinding, ModelSelector, ProtocolQuirks, ProviderClass, ProviderPluginDescriptor,
        TransportKind, VerificationStatus, WireProtocol, capability_for_operation,
    };
    use crate::config::ProviderConfig;

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
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

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn runtime_route_uses_provider_config_default_model_and_query_params() {
        let mut provider_config = ProviderConfig {
            base_url: Some("http://localhost:8080/v1".to_string()),
            default_model: Some("gpt-4.1".to_string()),
            ..ProviderConfig::default()
        };
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

    #[cfg(all(feature = "provider-openai", feature = "realtime"))]
    #[test]
    fn runtime_route_converts_openai_realtime_to_websocket_url() {
        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "openai",
                Some("gpt-realtime"),
                OperationKind::REALTIME_SESSION,
            ))
            .expect("official openai realtime route should resolve");

        assert_eq!(route.base_url, "wss://api.openai.com/v1");
        assert_eq!(
            route.url,
            "wss://api.openai.com/v1/realtime?model=gpt-realtime"
        );
        assert_eq!(route.transport(), TransportKind::WebSocket);
        assert_eq!(route.http_method(), None);
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
            behaviors: &[],
            capability_statuses: &[],
        };
        let registry = CatalogRegistry::new(&[PLUGIN]);

        let provider_config = ProviderConfig {
            base_url: Some("https://also-ignored.example.com/v1".to_string()),
            ..ProviderConfig::default()
        };

        let route = registry
            .resolve_runtime_route(
                RuntimeRouteRequest::new(
                    "zhipu-like",
                    Some("glm-realtime"),
                    OperationKind::REALTIME_SESSION,
                )
                .with_provider_config(&provider_config)
                .with_hints(InvocationHints {
                    streaming: Some(true),
                    ..InvocationHints::default()
                }),
            )
            .expect("endpoint override should win");

        assert_eq!(route.base_url, "wss://open.bigmodel.cn");
        assert_eq!(
            route.url,
            "wss://open.bigmodel.cn/api/paas/v4/realtime?model=glm-realtime"
        );
        assert!(route.invocation.quirks.require_model_prefix);
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
        const MODEL_REQUIRED_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "model-required",
            display_name: "Model Required",
            class: ProviderClass::Custom,
            default_base_url: Some("https://example.com/v1"),
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };
        const NO_BASE_URL_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
            id: "no-base-url",
            display_name: "No Base URL",
            class: ProviderClass::Custom,
            default_base_url: None,
            supported_auth: &[AuthMethodKind::ApiKeyHeader],
            auth_hint: None,
            models: &[],
            bindings: BINDINGS,
            behaviors: &[],
            capability_statuses: &[],
        };
        let registry = CatalogRegistry::new(&[MODEL_REQUIRED_PLUGIN, NO_BASE_URL_PLUGIN]);
        let err = registry
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "model-required",
                None,
                OperationKind::CHAT_COMPLETION,
            ))
            .expect_err("missing model should fail");
        assert!(matches!(
            err,
            crate::DittoError::ProviderResolution(
                crate::ProviderResolutionError::RuntimeRouteModelMissing
            )
        ));

        let err = registry
            .resolve_runtime_route(RuntimeRouteRequest::new(
                "no-base-url",
                Some("anything"),
                OperationKind::CHAT_COMPLETION,
            ))
            .expect_err("missing base url should fail");
        assert!(matches!(
            err,
            crate::DittoError::ProviderResolution(
                crate::ProviderResolutionError::RuntimeRouteBaseUrlMissing
            )
        ));
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
            provider_config.runtime_hints(),
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

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn runtime_route_rejects_required_capability_mismatch() {
        let err = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new(
                    "openai",
                    Some("text-embedding-3-large"),
                    OperationKind::CHAT_COMPLETION,
                )
                .with_required_capability(crate::catalog::CapabilityKind::LLM),
            )
            .expect_err("embedding-only model should fail llm capability constraint");
        assert!(matches!(
            err,
            crate::DittoError::ProviderResolution(
                crate::ProviderResolutionError::RuntimeRouteCapabilityUnsupported { .. }
            )
        ));
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn runtime_route_falls_back_to_generic_openai_compatible_plugin() {
        let provider_config = ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("custom-model".to_string()),
            ..ProviderConfig::default()
        };

        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new("acme-proxy", None, OperationKind::CHAT_COMPLETION)
                    .with_provider_config(&provider_config)
                    .with_required_capability(crate::catalog::CapabilityKind::LLM),
            )
            .expect("unknown openai-like provider should use generic catalog plugin");

        assert_eq!(route.invocation.provider, "openai-compatible");
        assert_eq!(route.url, "https://proxy.example/v1/chat/completions");
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn runtime_route_infers_official_base_url_for_known_openai_like_provider_hint() {
        let provider_config = ProviderConfig {
            default_model: Some("deepseek-chat".to_string()),
            ..ProviderConfig::default()
        };

        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new("deepseek", None, OperationKind::CHAT_COMPLETION)
                    .with_provider_config(&provider_config)
                    .with_required_capability(crate::catalog::CapabilityKind::LLM),
            )
            .expect("known openai-like provider hint should infer official base_url");

        #[cfg(feature = "provider-deepseek")]
        assert_eq!(route.invocation.provider, "deepseek");
        #[cfg(not(feature = "provider-deepseek"))]
        assert_eq!(route.invocation.provider, "openai-compatible");
        assert_eq!(route.base_url, "https://api.deepseek.com");
        assert_eq!(route.url, "https://api.deepseek.com/chat/completions");
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn runtime_route_can_resolve_custom_google_provider_from_upstream_api() {
        let provider_config = ProviderConfig {
            base_url: Some("https://yunwu.ai/v1beta".to_string()),
            default_model: Some("gemini-3.1-pro".to_string()),
            upstream_api: Some(crate::config::ProviderApi::GeminiGenerateContent),
            ..ProviderConfig::default()
        };

        let route = crate::catalog::builtin_registry()
            .resolve_runtime_route(
                RuntimeRouteRequest::new("yunwu", None, OperationKind::CHAT_COMPLETION)
                    .with_provider_config(&provider_config)
                    .with_required_capability(crate::catalog::CapabilityKind::LLM),
            )
            .expect("custom google provider should resolve via google catalog plugin");

        assert_eq!(route.invocation.provider, "google");
        assert!(route.url.ends_with(":generateContent"));
        assert!(route.url.starts_with("https://yunwu.ai/v1beta/"));
    }

    #[test]
    fn builtin_runtime_routes_stay_consistent_with_capabilities() {
        let registry = crate::catalog::builtin_registry();
        for plugin in registry.plugins() {
            let capability_bindings = plugin.capability_bindings();
            for model in plugin.models() {
                let mut provider_config = ProviderConfig::default();
                if plugin.default_base_url.is_none() {
                    provider_config.base_url = Some("https://example.invalid".to_string());
                }
                for &operation in model.supported_operations {
                    let Some(capability) = capability_for_operation(operation) else {
                        continue;
                    };
                    let Some(binding) = capability_bindings.iter().find(|binding| {
                        binding.capability == capability && binding.operations.contains(&operation)
                    }) else {
                        continue;
                    };
                    let route = match registry.resolve_runtime_route(
                        RuntimeRouteRequest::new(plugin.id, Some(model.id), operation)
                            .with_provider_config(&provider_config)
                            .with_required_capability(capability),
                    ) {
                        Ok(route) => route,
                        Err(crate::DittoError::ProviderResolution(
                            crate::ProviderResolutionError::CatalogRouteNotFound { .. },
                        )) => continue,
                        Err(err) => {
                            panic!(
                                "builtin catalog drifted for provider={} model={} operation={operation}: {err}",
                                plugin.id, model.id
                            )
                        }
                    };
                    assert!(binding.surfaces.contains(&route.invocation.surface));
                    assert!(
                        binding
                            .wire_protocols
                            .contains(&route.invocation.wire_protocol)
                    );
                }
            }
        }
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn runtime_route_keeps_invocation_metadata() {
        let provider_config = ProviderConfig {
            base_url: Some("https://example.com/v1".to_string()),
            default_model: Some("gpt-4.1".to_string()),
            http_query_params: BTreeMap::new(),
            ..ProviderConfig::default()
        };

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
