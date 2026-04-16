//! Runtime route planning and selection.
//!
//! `catalog` remains the source of static provider/model metadata. This module
//! owns the dynamic join step that combines catalog descriptors with request and
//! config hints to produce concrete runtime routes.

use super::route_endpoint::{
    adapt_runtime_base_url_for_transport, merge_runtime_query_params, resolve_runtime_base_url,
};
use super::route_selection::{
    effective_runtime_provider_hint, normalize_runtime_hints, resolve_runtime_model,
    resolve_runtime_plugin_and_source, validate_configured_enabled_capabilities,
};
use crate::catalog::{CatalogRegistry, ProviderCapabilityResolution, ProviderPluginDescriptor};
use crate::contracts::{
    CapabilityKind, InvocationHints, OperationKind, ProviderId, RuntimeProviderApi,
    RuntimeProviderHints, RuntimeRoute, RuntimeRouteRequest,
};
#[cfg(test)]
use crate::error::DittoError;
use crate::error::{ProviderResolutionError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeResolvedProviderSource {
    RequestProvider,
    ConfiguredProvider,
    UpstreamApiFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeResolvedModelSource {
    RequestModel,
    ProviderDefaultModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeResolvedBaseUrlSource {
    EndpointOverride,
    ProviderConfig,
    PluginDefault,
    InferredFromUpstreamApi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeRouteExplainPlan {
    pub provider_hint: String,
    pub resolved_provider: &'static str,
    pub provider_source: RuntimeResolvedProviderSource,
    pub requested_model: Option<String>,
    pub resolved_model: String,
    pub model_source: RuntimeResolvedModelSource,
    pub operation: OperationKind,
    pub required_capability: Option<CapabilityKind>,
    pub upstream_api: Option<RuntimeProviderApi>,
    pub normalized_hints: InvocationHints,
    pub capability_resolution: ProviderCapabilityResolution,
    pub origin_base_url: String,
    pub base_url_source: RuntimeResolvedBaseUrlSource,
    pub base_url_rewrite: Option<http_kit::WebsocketBaseUrlRewrite>,
    pub route: RuntimeRoute,
}

impl CatalogRegistry {
    pub(crate) fn plugin_for_runtime_request(
        &self,
        provider_name_hint: &str,
        provider_hints: RuntimeProviderHints<'_>,
    ) -> Option<&'static ProviderPluginDescriptor> {
        resolve_runtime_plugin_and_source(self, provider_name_hint, provider_hints)
            .map(|(plugin, _)| plugin)
    }
}

pub(crate) fn resolve_runtime_route(
    registry: &CatalogRegistry,
    request: RuntimeRouteRequest<'_>,
) -> Result<RuntimeRoute> {
    Ok(resolve_runtime_route_plan(registry, request)?.route)
}

pub(crate) fn resolve_runtime_route_plan(
    registry: &CatalogRegistry,
    request: RuntimeRouteRequest<'_>,
) -> Result<RuntimeRouteExplainPlan> {
    let provider = request.provider_id();
    let provider = ProviderId::new(provider.as_str().trim());
    let provider_hint = effective_runtime_provider_hint(provider.as_str(), request.provider_hints);
    if provider_hint.is_empty() {
        return Err(ProviderResolutionError::RuntimeRouteProviderMissing.into());
    }

    let (plugin, provider_source) =
        resolve_runtime_plugin_and_source(registry, provider.as_str(), request.provider_hints)
            .ok_or_else(|| ProviderResolutionError::CatalogProviderNotFound {
                provider: provider_hint.to_string(),
            })?;
    let requested_model = request
        .model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    let (model, model_source) = resolve_runtime_model(request.model, request.provider_hints)?;
    let capability_resolution = plugin.capability_resolution(Some(model.as_str()));
    if let Some(capability) = request.required_capability {
        validate_configured_enabled_capabilities(request.provider_hints, plugin.id, capability)?;
        if !capability_resolution.effective_supports(capability) {
            return Err(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: plugin.id.to_string(),
                model,
                capability: capability.to_string(),
            }
            .into());
        }
    }
    let hints = normalize_runtime_hints(request.operation, request.hints);
    let invocation = registry
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
    let (origin_base_url, base_url_source) = resolve_runtime_base_url(
        invocation.endpoint.base_url_override.as_deref(),
        request.provider_hints,
        plugin.default_base_url,
    )?;
    let (base_url, base_url_rewrite) = adapt_runtime_base_url_for_transport(
        origin_base_url.clone(),
        invocation.endpoint.transport,
    );
    let query_params =
        merge_runtime_query_params(&invocation.endpoint.query_params, request.provider_hints);
    let joined =
        http_kit::join_api_base_url_path(base_url.as_str(), invocation.endpoint.path.as_str());
    let url = http_kit::append_url_query_params(joined, &query_params);

    Ok(RuntimeRouteExplainPlan {
        provider_hint: provider_hint.to_string(),
        resolved_provider: plugin.id,
        provider_source,
        requested_model: requested_model.map(str::to_string),
        resolved_model: model.clone(),
        model_source,
        operation: request.operation,
        required_capability: request.required_capability,
        upstream_api: request.provider_hints.upstream_api,
        normalized_hints: hints,
        capability_resolution,
        origin_base_url,
        base_url_source,
        base_url_rewrite,
        route: RuntimeRoute {
            invocation,
            base_url,
            url,
            query_params,
        },
    })
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "provider-openai")]
    use std::collections::BTreeMap;

    use super::*;
    use crate::catalog::{
        ApiSurfaceId, AuthMethodKind, EndpointQueryParam, EndpointTemplate, HttpMethod,
        ModelBinding, ModelSelector, ProtocolQuirks, ProviderClass, ProviderPluginDescriptor,
        TransportKind, VerificationStatus, WireProtocol, capability_for_operation,
    };
    use crate::config::ProviderConfig;

    #[allow(dead_code)]
    fn resolve_builtin_route(request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
        resolve_runtime_route(&crate::catalog::builtin_registry(), request)
    }

    #[cfg(feature = "provider-openai")]
    #[test]
    fn runtime_route_uses_plugin_default_base_url() {
        let route = resolve_builtin_route(RuntimeRouteRequest::new(
            "openai",
            Some("gpt-4.1"),
            OperationKind::CHAT_COMPLETION,
        ))
        .expect("runtime route should resolve");

        assert_eq!(route.base_url, "https://api.openai.com/v1");
        assert_eq!(route.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(route.transport(), TransportKind::Http);
    }

    #[cfg(feature = "provider-openai")]
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

        let route = resolve_builtin_route(
            RuntimeRouteRequest::new("openai", None, OperationKind::CHAT_COMPLETION)
                .with_runtime_hints(provider_config.runtime_hints()),
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

    #[cfg(all(feature = "provider-openai", feature = "cap-realtime"))]
    #[test]
    fn runtime_route_converts_openai_realtime_to_websocket_url() {
        let route = resolve_builtin_route(RuntimeRouteRequest::new(
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

        let route = resolve_runtime_route(
            &registry,
            RuntimeRouteRequest::new(
                "zhipu-like",
                Some("glm-realtime"),
                OperationKind::REALTIME_SESSION,
            )
            .with_runtime_hints(provider_config.runtime_hints())
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
            http_kit::join_api_base_url_path("http://localhost:8080/v1", "/v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            http_kit::join_api_base_url_path("http://localhost:8080/v1", "v1/chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            http_kit::join_api_base_url_path("http://localhost:8080/v1", "/v1"),
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
        let err = resolve_runtime_route(
            &registry,
            RuntimeRouteRequest::new("model-required", None, OperationKind::CHAT_COMPLETION),
        )
        .expect_err("missing model should fail");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(ProviderResolutionError::RuntimeRouteModelMissing)
        ));

        let err = resolve_runtime_route(
            &registry,
            RuntimeRouteRequest::new(
                "no-base-url",
                Some("anything"),
                OperationKind::CHAT_COMPLETION,
            ),
        )
        .expect_err("missing base url should fail");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(ProviderResolutionError::RuntimeRouteBaseUrlMissing)
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
            http_kit::append_url_query_params(
                "https://example.com/path".to_string(),
                &[
                    ("a".to_string(), "1".to_string()),
                    ("b".to_string(), "2".to_string())
                ]
            ),
            "https://example.com/path?a=1&b=2"
        );
        assert_eq!(
            http_kit::append_url_query_params(
                "https://example.com/path?x=0".to_string(),
                &[("a".to_string(), "1".to_string())]
            ),
            "https://example.com/path?x=0&a=1"
        );
    }

    #[cfg(feature = "provider-openai")]
    #[test]
    fn runtime_route_rejects_required_capability_mismatch() {
        let err = resolve_builtin_route(
            RuntimeRouteRequest::new(
                "openai",
                Some("text-embedding-3-large"),
                OperationKind::CHAT_COMPLETION,
            )
            .with_required_capability(CapabilityKind::LLM),
        )
        .expect_err("embedding-only model should fail llm capability constraint");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(
                ProviderResolutionError::RuntimeRouteCapabilityUnsupported { .. }
            )
        ));
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn runtime_route_falls_back_to_generic_openai_compatible_plugin() {
        let provider_config = ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("custom-model".to_string()),
            ..ProviderConfig::default()
        };

        let route = resolve_builtin_route(
            RuntimeRouteRequest::new("acme-proxy", None, OperationKind::CHAT_COMPLETION)
                .with_runtime_hints(provider_config.runtime_hints())
                .with_required_capability(CapabilityKind::LLM),
        )
        .expect("unknown openai-like provider should use generic catalog plugin");

        assert_eq!(route.invocation.provider, "openai-compatible");
        assert_eq!(route.url, "https://proxy.example/v1/chat/completions");
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn runtime_route_requires_explicit_base_url_for_generic_openai_like_aliases() {
        let provider_config = ProviderConfig {
            default_model: Some("deepseek-chat".to_string()),
            ..ProviderConfig::default()
        };

        #[cfg(feature = "provider-deepseek")]
        {
            let route = resolve_builtin_route(
                RuntimeRouteRequest::new("deepseek", None, OperationKind::CHAT_COMPLETION)
                    .with_runtime_hints(provider_config.runtime_hints())
                    .with_required_capability(CapabilityKind::LLM),
            )
            .expect("dedicated deepseek catalog plugin should still resolve");

            assert_eq!(route.invocation.provider, "deepseek");
            assert_eq!(route.base_url, "https://api.deepseek.com");
            assert_eq!(route.url, "https://api.deepseek.com/chat/completions");
        }

        #[cfg(not(feature = "provider-deepseek"))]
        {
            let err = resolve_builtin_route(
                RuntimeRouteRequest::new("deepseek", None, OperationKind::CHAT_COMPLETION)
                    .with_runtime_hints(provider_config.runtime_hints())
                    .with_required_capability(CapabilityKind::LLM),
            )
            .expect_err(
                "generic openai-like aliases should require explicit base_url without catalog truth",
            );

            assert!(matches!(
                err,
                DittoError::ProviderResolution(ProviderResolutionError::RuntimeRouteBaseUrlMissing)
            ));
        }
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

        let route = resolve_builtin_route(
            RuntimeRouteRequest::new("yunwu", None, OperationKind::CHAT_COMPLETION)
                .with_runtime_hints(provider_config.runtime_hints())
                .with_required_capability(CapabilityKind::LLM),
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
                    let route = match resolve_runtime_route(
                        &registry,
                        RuntimeRouteRequest::new(plugin.id, Some(model.id), operation)
                            .with_runtime_hints(provider_config.runtime_hints())
                            .with_required_capability(capability),
                    ) {
                        Ok(route) => route,
                        Err(DittoError::ProviderResolution(
                            ProviderResolutionError::CatalogRouteNotFound { .. },
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

    #[cfg(feature = "provider-openai")]
    #[test]
    fn runtime_route_keeps_invocation_metadata() {
        let provider_config = ProviderConfig {
            base_url: Some("https://example.com/v1".to_string()),
            default_model: Some("gpt-4.1".to_string()),
            http_query_params: BTreeMap::new(),
            ..ProviderConfig::default()
        };

        let route = resolve_builtin_route(
            RuntimeRouteRequest::new("openai", None, OperationKind::CHAT_COMPLETION)
                .with_runtime_hints(provider_config.runtime_hints()),
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
