use super::route::{RuntimeResolvedModelSource, RuntimeResolvedProviderSource};
use crate::catalog::{CatalogRegistry, ProviderPluginDescriptor};
use crate::contracts::{
    CapabilityKind, InvocationHints, OperationKind, ProviderId, RuntimeProviderApi,
    RuntimeProviderHints,
};
use crate::error::{ProviderResolutionError, Result};

pub(super) fn resolve_runtime_plugin_and_source(
    registry: &CatalogRegistry,
    provider_name_hint: &str,
    provider_hints: RuntimeProviderHints<'_>,
) -> Option<(
    &'static ProviderPluginDescriptor,
    RuntimeResolvedProviderSource,
)> {
    let provider = provider_name_hint.trim();
    if let Some(plugin) = (!provider.is_empty())
        .then(|| {
            registry
                .plugin_by_id(ProviderId::new(provider))
                .or_else(|| registry.plugin_by_hint(provider))
        })
        .flatten()
    {
        return Some((plugin, RuntimeResolvedProviderSource::RequestProvider));
    }

    if let Some(configured_provider) = configured_provider_hint(provider_hints)
        && let Some(plugin) = registry
            .plugin_by_id(ProviderId::new(configured_provider))
            .or_else(|| registry.plugin_by_hint(configured_provider))
    {
        return Some((plugin, RuntimeResolvedProviderSource::ConfiguredProvider));
    }

    fallback_runtime_plugin(registry, provider_hints)
        .map(|plugin| (plugin, RuntimeResolvedProviderSource::UpstreamApiFallback))
}

pub(super) fn effective_runtime_provider_hint<'a>(
    provider_name_hint: &'a str,
    provider_hints: RuntimeProviderHints<'a>,
) -> &'a str {
    let provider = provider_name_hint.trim();
    if !provider.is_empty() {
        return provider;
    }
    configured_provider_hint(provider_hints).unwrap_or("")
}

pub(super) fn validate_configured_enabled_capabilities(
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

pub(super) fn normalize_runtime_hints(
    operation: OperationKind,
    mut hints: InvocationHints,
) -> InvocationHints {
    if operation == OperationKind::REALTIME_SESSION && hints.streaming.is_none() {
        hints.streaming = Some(true);
    }
    hints
}

pub(super) fn resolve_runtime_model(
    explicit_model: Option<&str>,
    provider_hints: RuntimeProviderHints<'_>,
) -> Result<(String, RuntimeResolvedModelSource)> {
    if let Some(model) = explicit_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok((model.to_string(), RuntimeResolvedModelSource::RequestModel));
    }

    if let Some(model) = provider_hints
        .default_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Ok((
            model.to_string(),
            RuntimeResolvedModelSource::ProviderDefaultModel,
        ));
    }

    Err(ProviderResolutionError::RuntimeRouteModelMissing.into())
}

fn configured_provider_hint(provider_hints: RuntimeProviderHints<'_>) -> Option<&str> {
    provider_hints
        .configured_provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
