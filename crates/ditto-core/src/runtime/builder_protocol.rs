//! Runtime builder assembly protocol.
//!
//! This module owns the internal protocol that resolves provider/config input
//! into a concrete builder backend and effective runtime config. `model_builders`
//! consumes the plan, but does not own the resolution contract itself.

use super::builtin::builtin_runtime_assembly;
use super::route::resolve_runtime_route;
use crate::config::ProviderConfig;
use crate::contracts::{
    CapabilityKind, RuntimeRoute, RuntimeRouteRequest, invocation_operations_for_capability,
};
use crate::error::{DittoError, Result};

// RUNTIME-BUILDER-ASSEMBLY-PROTOCOL: keep provider/config/capability ->
// builder-backend/config resolution in one owner so `model_builders` stays
// focused on adapter instantiation instead of accumulating route semantics.

fn unsupported_provider_backend(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.unsupported_provider_backend",
        "provider" => provider
    )
}

fn route_resolution_failed(
    provider: &str,
    model: &str,
    capability: CapabilityKind,
    attempts: usize,
) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.route_resolution_failed",
        "provider" => provider,
        "model" => model,
        "capability" => capability.to_string(),
        "attempts" => attempts.to_string()
    )
}

fn context_cache_model_missing(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.context_cache_model_missing_config_hint",
        "provider" => provider
    )
}

pub(super) fn configured_default_model(config: &ProviderConfig) -> Option<&str> {
    config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BuilderAssemblyRequest<'a> {
    provider: &'a str,
    config: &'a ProviderConfig,
    capability: CapabilityKind,
}

impl<'a> BuilderAssemblyRequest<'a> {
    pub(super) const fn new(
        provider: &'a str,
        config: &'a ProviderConfig,
        capability: CapabilityKind,
    ) -> Self {
        Self {
            provider,
            config,
            capability,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct BuilderAssemblyPlan {
    pub(super) provider: &'static str,
    pub(super) behavior_provider: &'static str,
    pub(super) config: ProviderConfig,
}

fn apply_runtime_route_to_builder_config(
    config: &ProviderConfig,
    route: &RuntimeRoute,
) -> ProviderConfig {
    let mut runtime_config = config.clone();
    runtime_config.base_url = Some(route.base_url.clone());
    runtime_config.default_model = Some(route.invocation.model.clone());
    runtime_config
}

pub(super) fn default_builder_assembly(
    provider: &str,
    config: &ProviderConfig,
) -> Result<BuilderAssemblyPlan> {
    let runtime = builtin_runtime_assembly();
    let provider = provider.trim();
    if provider.is_empty() {
        return Err(unsupported_provider_backend(provider));
    }

    let plugin = runtime
        .registry()
        .resolve_builder_provider(provider, config)
        .ok_or_else(|| unsupported_provider_backend(provider))?;

    let mut runtime_config = config.clone();
    if runtime_config.base_url.is_none() {
        runtime_config.base_url = plugin.default_base_url.map(str::to_string);
    }

    Ok(BuilderAssemblyPlan {
        provider: plugin.builder_provider,
        behavior_provider: plugin.catalog_provider,
        config: runtime_config,
    })
}

pub(super) fn resolve_builder_assembly(
    request: BuilderAssemblyRequest<'_>,
) -> Result<BuilderAssemblyPlan> {
    let runtime = builtin_runtime_assembly();
    let fallback = default_builder_assembly(request.provider, request.config)?;
    let Some(plugin) = runtime
        .registry()
        .resolve_builder_provider(request.provider.trim(), request.config)
    else {
        return Ok(fallback);
    };

    let requested_model = if request.capability == CapabilityKind::BATCH {
        None
    } else {
        configured_default_model(request.config)
    };

    if let Some(model) = requested_model {
        let Some(probe_operations) = invocation_operations_for_capability(request.capability)
        else {
            if !runtime.registry().provider_supports_capability(
                request.provider,
                request.config,
                Some(model),
                request.capability,
            ) {
                return Err(
                    crate::error::ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                        provider: plugin.catalog_provider.to_string(),
                        model: model.to_string(),
                        capability: request.capability.to_string(),
                    }
                    .into(),
                );
            }
            return Ok(fallback);
        };

        let mut first_error = None;
        let mut error_count = 0usize;
        for &operation in probe_operations {
            match resolve_runtime_route(
                &runtime.catalog(),
                RuntimeRouteRequest::new(plugin.catalog_provider, Some(model), operation)
                    .with_runtime_hints(request.config.runtime_hints())
                    .with_required_capability(request.capability),
            ) {
                Ok(route) => {
                    let runtime_config =
                        apply_runtime_route_to_builder_config(request.config, &route);
                    let resolved = runtime
                        .registry()
                        .resolve_builder_provider(route.invocation.provider, &runtime_config);
                    return Ok(BuilderAssemblyPlan {
                        provider: resolved
                            .map(|resolution| resolution.builder_provider)
                            .unwrap_or(fallback.provider),
                        behavior_provider: resolved
                            .map(|resolution| resolution.catalog_provider)
                            .unwrap_or(fallback.behavior_provider),
                        config: runtime_config,
                    });
                }
                Err(err) => {
                    error_count += 1;
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
            }
        }

        if error_count <= 1 {
            return Err(first_error.expect("builder route resolution should record an error"));
        }

        return Err(route_resolution_failed(
            plugin.catalog_provider,
            model,
            request.capability,
            error_count,
        ));
    }

    if !runtime.registry().provider_supports_capability(
        request.provider,
        request.config,
        None,
        request.capability,
    ) {
        return Err(
            crate::error::ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: plugin.catalog_provider.to_string(),
                model: "*".to_string(),
                capability: request.capability.to_string(),
            }
            .into(),
        );
    }

    Ok(fallback)
}

// RUNTIME-CONTEXT-CACHE-ASSEMBLY-PROTOCOL: context cache does not resolve via
// invocation route planning, so keep its support/default-model checks in the
// assembly owner instead of leaking registry semantics back into frontdoors.
pub(super) fn resolve_context_cache_assembly(
    provider: &str,
    config: &ProviderConfig,
) -> Result<BuilderAssemblyPlan> {
    let runtime = builtin_runtime_assembly();
    let provider = provider.trim();
    let plugin = runtime
        .catalog()
        .plugin(provider)
        .or_else(|| {
            runtime
                .catalog()
                .plugin_for_runtime_request(provider, config.runtime_hints())
        })
        .ok_or_else(|| unsupported_provider_backend(provider))?;
    let mut runtime_config = config.clone();
    if runtime_config.base_url.is_none() {
        runtime_config.base_url = plugin.default_base_url.map(str::to_string);
    }
    let fallback = BuilderAssemblyPlan {
        provider: plugin.id,
        behavior_provider: plugin.id,
        config: runtime_config,
    };
    let model = configured_default_model(config)
        .ok_or_else(|| context_cache_model_missing(fallback.behavior_provider))?;

    if !runtime.registry().provider_supports_capability(
        provider,
        config,
        Some(model),
        CapabilityKind::CONTEXT_CACHE,
    ) {
        return Err(
            crate::error::ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: fallback.behavior_provider.to_string(),
                model: model.to_string(),
                capability: CapabilityKind::CONTEXT_CACHE.to_string(),
            }
            .into(),
        );
    }

    Ok(fallback)
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[cfg(feature = "provider-openai")]
    #[test]
    fn builder_assembly_accepts_response_only_openai_model() {
        let runtime = resolve_builder_assembly(BuilderAssemblyRequest::new(
            "openai",
            &ProviderConfig {
                base_url: Some("https://api.openai.com/v1".to_string()),
                default_model: Some("computer-use-preview".to_string()),
                ..ProviderConfig::default()
            },
            CapabilityKind::LLM,
        ))
        .expect("response-only openai model should resolve");

        assert_eq!(runtime.provider, "openai");
        assert_eq!(
            runtime.config.default_model.as_deref(),
            Some("computer-use-preview")
        );
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
    }

    #[cfg(feature = "provider-deepseek")]
    #[test]
    fn builder_assembly_infers_deepseek_base_url_from_catalog_route() {
        let runtime = resolve_builder_assembly(BuilderAssemblyRequest::new(
            "deepseek",
            &ProviderConfig {
                default_model: Some("deepseek-reasoner".to_string()),
                ..ProviderConfig::default()
            },
            CapabilityKind::LLM,
        ))
        .expect("deepseek runtime should resolve");

        assert_eq!(runtime.provider, "openai-compatible");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            runtime.config.default_model.as_deref(),
            Some("deepseek-reasoner")
        );
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn builder_assembly_rejects_unknown_custom_provider_without_explicit_owner() {
        let err = resolve_builder_assembly(BuilderAssemblyRequest::new(
            "yunwu-openai",
            &ProviderConfig {
                base_url: Some("https://proxy.example/v1".to_string()),
                default_model: Some("chat-model".to_string()),
                ..ProviderConfig::default()
            },
            CapabilityKind::LLM,
        ))
        .expect_err("unknown custom provider should fail closed");

        assert!(
            err.to_string()
                .contains("unsupported provider backend: yunwu-openai")
        );
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn builder_assembly_uses_explicit_configured_provider_for_custom_node() {
        let runtime = resolve_builder_assembly(BuilderAssemblyRequest::new(
            "yunwu-openai",
            &ProviderConfig {
                provider: Some("openai-compatible".to_string()),
                base_url: Some("https://proxy.example/v1".to_string()),
                default_model: Some("chat-model".to_string()),
                ..ProviderConfig::default()
            },
            CapabilityKind::LLM,
        ))
        .expect("explicit configured provider should keep generic openai-compatible runtime");

        assert_eq!(runtime.provider, "openai-compatible");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://proxy.example/v1")
        );
        assert_eq!(runtime.config.default_model.as_deref(), Some("chat-model"));
    }

    #[cfg(feature = "provider-deepseek")]
    #[test]
    fn context_cache_assembly_resolves_deepseek_without_invocation_route_planning() {
        let runtime = resolve_context_cache_assembly(
            "deepseek",
            &ProviderConfig {
                default_model: Some("deepseek-chat".to_string()),
                ..ProviderConfig::default()
            },
        )
        .expect("deepseek context cache assembly should resolve");

        assert_eq!(runtime.provider, "deepseek");
        assert_eq!(runtime.behavior_provider, "deepseek");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            runtime.config.default_model.as_deref(),
            Some("deepseek-chat")
        );
    }

    #[cfg(feature = "provider-minimax")]
    #[test]
    fn context_cache_assembly_keeps_minimax_under_catalog_owner() {
        let runtime = resolve_context_cache_assembly(
            "minimax",
            &ProviderConfig {
                provider: Some("minimax".to_string()),
                default_model: Some("MiniMax-M2".to_string()),
                ..ProviderConfig::default()
            },
        )
        .expect("minimax context cache assembly should resolve");

        assert_eq!(runtime.provider, "minimax");
        assert_eq!(runtime.behavior_provider, "minimax");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.minimaxi.com")
        );
        assert_eq!(runtime.config.default_model.as_deref(), Some("MiniMax-M2"));
    }
}
