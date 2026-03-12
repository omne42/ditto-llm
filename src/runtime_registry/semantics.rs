use super::catalog::{
    BuiltinRuntimeRegistryCatalog, ResolvedBuiltinBuilderProvider,
    ResolvedProviderCapabilityProfile, ResolvedProviderConfigSemantics,
};
use crate::capabilities::context_cache::{ContextCacheMode, ContextCacheProfile};
use crate::catalog::{
    AssistantToolFollowupRequirement, BehaviorSupport, CatalogRegistry,
    ProviderCapabilityResolution, ProviderPluginDescriptor,
};
use crate::config::{ProviderApi, ProviderCapabilities, ProviderConfig};
use crate::contracts::{
    CapabilityKind, ContextCacheModeId, InvocationHints, OperationKind, ProviderClass, WireProtocol,
};
use crate::foundation::error::{ProviderResolutionError, Result};

impl BuiltinRuntimeRegistryCatalog {
    pub fn resolve_provider_config_semantics(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
    ) -> Result<ResolvedProviderConfigSemantics> {
        let plugin = resolve_configured_catalog_plugin(
            self.registry(),
            provider_name_hint,
            provider_config,
        )?;
        let runtime_capabilities = plugin.runtime_spec().capabilities;
        let mut enabled_capabilities = if provider_config.enabled_capabilities.is_empty() {
            infer_enabled_capabilities(plugin, provider_config)
        } else {
            parse_enabled_capability_list(&provider_config.enabled_capabilities)?
        };
        enabled_capabilities.sort_by_key(|capability| capability.as_str());
        enabled_capabilities.dedup();

        for capability in &enabled_capabilities {
            if !runtime_capabilities.contains(*capability) {
                return Err(ProviderResolutionError::ConfiguredCapabilityUnsupported {
                    provider: plugin.id.to_string(),
                    capability: capability.to_string(),
                }
                .into());
            }
        }

        if provider_config.capabilities.is_some()
            && !enabled_capabilities.contains(&CapabilityKind::LLM)
        {
            let scope = provider_config
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("provider={} model={value}", plugin.id))
                .unwrap_or_else(|| format!("provider={}", plugin.id));
            return Err(ProviderResolutionError::ProviderCapabilitiesRequireLlm { scope }.into());
        }

        Ok(ResolvedProviderConfigSemantics {
            provider: plugin.id,
            provider_display_name: plugin.display_name,
            enabled_capabilities,
        })
    }

    pub fn resolve_openai_compatible_provider_capability_profile(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
    ) -> Result<ResolvedProviderCapabilityProfile> {
        let plugin = resolve_openai_compatible_catalog_plugin(
            self.registry(),
            provider_name_hint,
            provider_config,
        )?;
        let model = provider_config
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let resolution = plugin.capability_resolution(model);

        if provider_config.capabilities.is_some()
            && !resolution.effective_supports(CapabilityKind::LLM)
        {
            let scope = resolution
                .requested_model
                .as_deref()
                .map(|value| format!("provider={} model={value}", plugin.id))
                .unwrap_or_else(|| format!("provider={}", plugin.id));
            return Err(ProviderResolutionError::ProviderCapabilitiesRequireLlm { scope }.into());
        }

        let configured_capabilities = provider_config.capabilities;
        let effective_capabilities = configured_capabilities
            .unwrap_or_else(|| catalog_default_provider_capabilities(&resolution));

        Ok(ResolvedProviderCapabilityProfile {
            provider: plugin.id,
            provider_display_name: plugin.display_name,
            resolution,
            configured_capabilities,
            effective_capabilities,
        })
    }

    pub(crate) fn resolve_builder_provider(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
    ) -> Option<ResolvedBuiltinBuilderProvider> {
        let plugin =
            resolve_builder_catalog_plugin(self.registry(), provider_name_hint, provider_config)?;
        Some(ResolvedBuiltinBuilderProvider {
            catalog_provider: plugin.id,
            builder_provider: canonical_builder_provider_from_plugin(plugin)?,
            default_base_url: plugin.default_base_url,
        })
    }

    pub(crate) fn provider_supports_capability(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
        model: Option<&str>,
        capability: CapabilityKind,
    ) -> bool {
        let Some(plugin) =
            resolve_builder_catalog_plugin(self.registry(), provider_name_hint, provider_config)
        else {
            return false;
        };

        let requested_model = if capability == CapabilityKind::BATCH {
            None
        } else {
            model.map(str::trim).filter(|value| !value.is_empty())
        };

        plugin
            .capability_resolution(requested_model)
            .effective_supports(capability)
    }

    pub fn validate_runtime_route_for_provider_config(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
        model: &str,
        operation: OperationKind,
        required_capability: CapabilityKind,
    ) -> Result<()> {
        let provider = provider_name_hint.trim();
        if provider.is_empty() {
            return Err(ProviderResolutionError::RuntimeRouteProviderMissing.into());
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(ProviderResolutionError::RuntimeRouteModelMissing.into());
        }

        let plugin =
            resolve_builder_catalog_plugin(self.registry(), provider_name_hint, provider_config)
                .ok_or_else(|| ProviderResolutionError::CatalogProviderNotFound {
                    provider: provider.to_string(),
                })?;

        validate_runtime_enabled_capabilities(provider_config, plugin.id, required_capability)?;

        let capability_resolution = plugin.capability_resolution(Some(model));
        if !capability_resolution.effective_supports(required_capability) {
            return Err(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: plugin.id.to_string(),
                model: model.to_string(),
                capability: required_capability.to_string(),
            }
            .into());
        }

        let hints = normalized_runtime_validation_hints(operation);
        let invocation = self
            .registry()
            .resolve_with_hints_for_provider(plugin.provider_id(), model, operation, hints)
            .ok_or_else(|| ProviderResolutionError::CatalogRouteNotFound {
                provider: plugin.id.to_string(),
                model: model.to_string(),
                operation: operation.to_string(),
            })?;

        validate_runtime_base_url_available(
            invocation.endpoint.base_url_override.as_deref(),
            provider_config,
            plugin.default_base_url,
        )?;

        Ok(())
    }

    pub(crate) fn resolve_context_cache_profile(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
        model: &str,
    ) -> Option<ContextCacheProfile> {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }

        let plugin =
            resolve_builder_catalog_plugin(self.registry(), provider_name_hint, provider_config)?;
        resolve_catalog_context_cache_profile(plugin, model)
    }

    pub(crate) fn resolve_catalog_context_cache_profile(
        self,
        catalog_provider: &str,
        model: &str,
    ) -> Option<ContextCacheProfile> {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }

        let plugin = self.registry().plugin_by_hint(catalog_provider)?;
        resolve_catalog_context_cache_profile(plugin, model)
    }

    #[allow(dead_code)]
    pub(crate) fn provider_supports_operation(
        self,
        provider_name_hint: &str,
        model: &str,
        operation: OperationKind,
    ) -> bool {
        let model = model.trim();
        if model.is_empty() {
            return false;
        }

        self.registry()
            .plugin_by_hint(provider_name_hint)
            .and_then(|plugin| plugin.resolve(model, operation))
            .is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn provider_supports_file_builder(
        self,
        provider_name_hint: &str,
        provider_config: &ProviderConfig,
    ) -> bool {
        matches!(
            self.resolve_builder_provider(provider_name_hint, provider_config)
                .map(|resolved| resolved.builder_provider),
            Some("openai" | "openai-compatible")
        )
    }

    pub(crate) fn provider_requires_reasoning_content_followup(
        self,
        provider_name_hint: &str,
        model: &str,
        operation: OperationKind,
    ) -> bool {
        matches!(
            self.registry()
                .plugin_by_hint(provider_name_hint)
                .and_then(|plugin| plugin.behavior(model, operation))
                .map(|behavior| behavior.assistant_tool_followup),
            Some(AssistantToolFollowupRequirement::RequiresReasoningContent)
        )
    }

    pub(crate) fn provider_required_tool_choice_support(
        self,
        provider_name_hint: &str,
        model: &str,
        operation: OperationKind,
    ) -> Option<bool> {
        match self
            .registry()
            .plugin_by_hint(provider_name_hint)
            .and_then(|plugin| plugin.behavior(model, operation))
            .map(|behavior| behavior.tool_choice_required)
        {
            Some(BehaviorSupport::Supported) => Some(true),
            Some(BehaviorSupport::Unsupported) => Some(false),
            _ => None,
        }
    }
}

fn resolve_openai_compatible_catalog_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<&'static ProviderPluginDescriptor> {
    let plugin = resolve_configured_catalog_plugin(registry, provider_name_hint, provider_config)?;
    match plugin.class {
        ProviderClass::GenericOpenAi | ProviderClass::OpenAiCompatible => Ok(plugin),
        other => Err(ProviderResolutionError::UnsupportedProviderClass {
            provider_hint: provider_name_hint.to_string(),
            resolved_provider: plugin.id.to_string(),
            resolved_class: format!("{other:?}"),
        }
        .into()),
    }
}

fn resolve_builder_catalog_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Option<&'static ProviderPluginDescriptor> {
    let provider = provider_name_hint.trim();
    if provider.is_empty() {
        return builder_plugin_from_upstream_api(registry, provider_config);
    }

    resolve_explicit_catalog_plugin(registry, provider)
        .or_else(|| resolve_configured_catalog_plugin_hint(registry, provider_config))
}

fn resolve_configured_catalog_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<&'static ProviderPluginDescriptor> {
    if let Some(provider) = provider_config
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return resolve_explicit_catalog_plugin(registry, provider).ok_or_else(|| {
            ProviderResolutionError::ConfiguredProviderNotFound {
                provider: provider.to_string(),
            }
            .into()
        });
    }

    let provider = provider_name_hint.trim();
    if provider.is_empty() {
        return builder_plugin_from_upstream_api(registry, provider_config).ok_or_else(|| {
            ProviderResolutionError::CatalogProviderNotFound {
                provider: provider.to_string(),
            }
            .into()
        });
    }

    resolve_explicit_catalog_plugin(registry, provider).ok_or_else(|| {
        ProviderResolutionError::CatalogProviderNotFound {
            provider: provider.to_string(),
        }
        .into()
    })
}

fn resolve_explicit_catalog_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
) -> Option<&'static ProviderPluginDescriptor> {
    let provider = provider_name_hint.trim();
    if provider.is_empty() {
        return None;
    }

    registry
        .plugin(provider)
        .or_else(|| namespaced_catalog_plugin(registry, provider))
        .or_else(|| legacy_builder_alias_plugin(registry, provider))
}

fn resolve_configured_catalog_plugin_hint(
    registry: CatalogRegistry,
    provider_config: &ProviderConfig,
) -> Option<&'static ProviderPluginDescriptor> {
    provider_config
        .provider
        .as_deref()
        .and_then(|provider| resolve_explicit_catalog_plugin(registry, provider))
}

fn namespaced_catalog_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
) -> Option<&'static ProviderPluginDescriptor> {
    let (namespace, _) = provider_name_hint.split_once(".providers.")?;
    resolve_explicit_catalog_plugin(registry, namespace)
}

fn legacy_builder_alias_plugin(
    registry: CatalogRegistry,
    provider_name_hint: &str,
) -> Option<&'static ProviderPluginDescriptor> {
    let generic_openai = || {
        registry
            .plugin("openai-compatible")
            .or_else(|| registry.plugin("openai"))
    };

    match normalized_provider_alias(provider_name_hint).as_str() {
        "openaicompatible" | "litellm" | "azure" | "azureopenai" | "groq" | "mistral"
        | "together" | "togetherai" | "fireworks" | "perplexity" | "ollama" | "qwen" => {
            generic_openai()
        }
        "openrouter" => registry.plugin("openrouter").or_else(generic_openai),
        "deepseek" => registry.plugin("deepseek").or_else(generic_openai),
        "moonshot" | "moonshotai" | "kimi" => registry.plugin("kimi").or_else(generic_openai),
        "minimax" => registry.plugin("minimax").or_else(generic_openai),
        "glm" | "zhipu" => registry.plugin("zhipu").or_else(generic_openai),
        "doubao" | "ark" => registry.plugin("doubao").or_else(generic_openai),
        "xai" | "grok" => registry.plugin("xai").or_else(generic_openai),
        _ => None,
    }
}

fn normalized_provider_alias(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn parse_enabled_capability_list(enabled_capabilities: &[String]) -> Result<Vec<CapabilityKind>> {
    let mut out = Vec::new();
    for capability in enabled_capabilities {
        let Some(parsed) = CapabilityKind::parse_config_token(capability) else {
            return Err(ProviderResolutionError::ConfiguredCapabilityUnknown {
                capability: capability.trim().to_string(),
            }
            .into());
        };
        if !out.contains(&parsed) {
            out.push(parsed);
        }
    }
    Ok(out)
}

fn infer_enabled_capabilities(
    plugin: &ProviderPluginDescriptor,
    provider_config: &ProviderConfig,
) -> Vec<CapabilityKind> {
    if provider_config.capabilities.is_some() {
        return vec![CapabilityKind::LLM];
    }

    if let Some(upstream_api) = provider_config.upstream_api {
        return vec![capability_for_provider_api(upstream_api)];
    }

    if let Some(model) = provider_config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mut inferred: Vec<_> = plugin
            .capability_resolution(Some(model))
            .effective_capabilities
            .iter()
            .collect();
        inferred.sort_by_key(|capability| capability.as_str());
        inferred.dedup();
        if !inferred.is_empty() {
            return inferred;
        }
    }

    let mut inferred: Vec<_> = plugin.runtime_spec().capabilities.iter().collect();
    inferred.sort_by_key(|capability| capability.as_str());
    inferred.dedup();
    inferred
}

fn capability_for_provider_api(provider_api: ProviderApi) -> CapabilityKind {
    match provider_api {
        ProviderApi::OpenaiChatCompletions
        | ProviderApi::OpenaiResponses
        | ProviderApi::GeminiGenerateContent
        | ProviderApi::AnthropicMessages => CapabilityKind::LLM,
    }
}

fn validate_runtime_enabled_capabilities(
    provider_config: &ProviderConfig,
    provider: &str,
    required_capability: CapabilityKind,
) -> Result<()> {
    if provider_config.enabled_capabilities.is_empty() {
        return Ok(());
    }

    let mut configured_supports_required = false;
    for capability in &provider_config.enabled_capabilities {
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
        model: provider_config
            .default_model
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| "<unspecified>".to_string()),
        capability: required_capability.to_string(),
    }
    .into())
}

fn normalized_runtime_validation_hints(operation: OperationKind) -> InvocationHints {
    let mut hints = InvocationHints::default();
    if operation == OperationKind::REALTIME_SESSION {
        hints.streaming = Some(true);
    }
    hints
}

fn validate_runtime_base_url_available(
    endpoint_override: Option<&str>,
    provider_config: &ProviderConfig,
    plugin_default_base_url: Option<&str>,
) -> Result<()> {
    if endpoint_override
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }

    if provider_config
        .base_url
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }

    if plugin_default_base_url
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }

    match provider_config.upstream_api {
        Some(ProviderApi::GeminiGenerateContent) | Some(ProviderApi::AnthropicMessages) => Ok(()),
        Some(ProviderApi::OpenaiChatCompletions) | Some(ProviderApi::OpenaiResponses) | None => {
            Err(ProviderResolutionError::RuntimeRouteBaseUrlMissing.into())
        }
    }
}

fn builder_plugin_from_upstream_api(
    registry: CatalogRegistry,
    provider_config: &ProviderConfig,
) -> Option<&'static ProviderPluginDescriptor> {
    match provider_config.upstream_api {
        Some(ProviderApi::GeminiGenerateContent) => registry.plugin("google"),
        Some(ProviderApi::AnthropicMessages) => registry.plugin("anthropic"),
        Some(ProviderApi::OpenaiChatCompletions) | Some(ProviderApi::OpenaiResponses) | None => {
            registry
                .plugin("openai-compatible")
                .or_else(|| registry.plugin("openai"))
        }
    }
}

fn canonical_builder_provider_from_plugin(
    plugin: &ProviderPluginDescriptor,
) -> Option<&'static str> {
    match plugin.id {
        "openai" => Some("openai"),
        "anthropic" => Some("anthropic"),
        "google" => Some("google"),
        "cohere" => Some("cohere"),
        "bedrock" => Some("bedrock"),
        "vertex" => Some("vertex"),
        _ => match plugin.class {
            ProviderClass::GenericOpenAi | ProviderClass::OpenAiCompatible => {
                Some("openai-compatible")
            }
            _ => None,
        },
    }
}

fn catalog_default_provider_capabilities(
    resolution: &ProviderCapabilityResolution,
) -> ProviderCapabilities {
    if !resolution.effective_supports(CapabilityKind::LLM) {
        return ProviderCapabilities::disabled();
    }

    ProviderCapabilities::catalog_default_llm(
        resolution.model_is_catalog_known()
            && resolution.effective_supports(CapabilityKind::CONTEXT_CACHE),
    )
}

fn push_context_cache_mode(modes: &mut Vec<ContextCacheMode>, mode: ContextCacheMode) {
    if !modes.contains(&mode) {
        modes.push(mode);
    }
}

fn extend_context_cache_modes(
    modes: &mut Vec<ContextCacheMode>,
    configured_modes: &[ContextCacheModeId],
) {
    for mode in configured_modes {
        match *mode {
            ContextCacheModeId::PASSIVE => {
                push_context_cache_mode(modes, ContextCacheMode::Passive);
            }
            ContextCacheModeId::PROMPT_CACHE_KEY => {
                push_context_cache_mode(modes, ContextCacheMode::PromptCacheKey);
            }
            ContextCacheModeId::ANTHROPIC_COMPATIBLE => {
                push_context_cache_mode(modes, ContextCacheMode::AnthropicCompatible);
            }
            _ => {}
        }
    }
}

// RUNTIME-CONTEXT-CACHE-PROFILE-OWNER: registry semantics own the static
// context-cache contract so runtime builders and provider profiles consume one
// machine-readable source instead of re-encoding provider-name heuristics.
fn resolve_catalog_context_cache_profile(
    plugin: &ProviderPluginDescriptor,
    model: &str,
) -> Option<ContextCacheProfile> {
    if !plugin
        .capability_resolution(Some(model))
        .effective_supports(CapabilityKind::CONTEXT_CACHE)
    {
        return None;
    }

    let canonical_model = plugin.model(model).map(|entry| entry.id).unwrap_or(model);
    let mut modes = Vec::<ContextCacheMode>::new();
    let mut notes = Vec::<&'static str>::new();

    for operation in [OperationKind::CHAT_COMPLETION, OperationKind::CONTEXT_CACHE] {
        if let Some(behavior) = plugin.behavior(canonical_model, operation) {
            extend_context_cache_modes(&mut modes, behavior.context_cache_modes);
            if let Some(note) = behavior.notes.filter(|value| !value.trim().is_empty())
                && !notes.contains(&note)
            {
                notes.push(note);
            }
        }
    }

    let mut has_context_cache_binding = false;
    let mut has_anthropic_context_cache_binding = false;
    for binding in plugin.bindings {
        if !binding.matches(
            canonical_model,
            OperationKind::CONTEXT_CACHE,
            InvocationHints::default(),
        ) {
            continue;
        }
        has_context_cache_binding = true;
        if binding.wire_protocol == WireProtocol::ANTHROPIC_MESSAGES {
            has_anthropic_context_cache_binding = true;
        }
    }

    if has_context_cache_binding {
        push_context_cache_mode(&mut modes, ContextCacheMode::Passive);
    }
    if has_anthropic_context_cache_binding {
        push_context_cache_mode(&mut modes, ContextCacheMode::AnthropicCompatible);
    }

    if modes.is_empty() {
        return None;
    }

    Some(ContextCacheProfile {
        modes,
        notes: (!notes.is_empty()).then(|| notes.join(" ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_registry::builtin_runtime_registry_catalog;

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn rejects_unknown_openai_like_provider_without_explicit_owner() {
        let config = ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("custom-model".to_string()),
            ..ProviderConfig::default()
        };

        let err = builtin_runtime_registry_catalog()
            .resolve_openai_compatible_provider_capability_profile("my-proxy", &config)
            .expect_err("unknown provider should fail closed without explicit owner");
        assert!(matches!(
            err,
            crate::foundation::error::DittoError::ProviderResolution(
                crate::foundation::error::ProviderResolutionError::CatalogProviderNotFound { .. }
            )
        ));
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn resolves_explicit_openai_compatible_owner_for_custom_provider_node() {
        let config = ProviderConfig {
            provider: Some("openai-compatible".to_string()),
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("custom-model".to_string()),
            ..ProviderConfig::default()
        };

        let profile = builtin_runtime_registry_catalog()
            .resolve_openai_compatible_provider_capability_profile("my-proxy", &config)
            .expect("explicit configured provider should resolve generic openai-compatible");
        assert_eq!(profile.provider, "openai-compatible");
        assert!(profile.resolution.effective_supports(CapabilityKind::LLM));
        assert!(profile.effective_capabilities.streaming);
        assert!(!profile.effective_capabilities.prompt_cache);
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn resolve_builtin_builder_provider_maps_gemini_upstream_to_google() {
        let config = ProviderConfig {
            upstream_api: Some(ProviderApi::GeminiGenerateContent),
            ..ProviderConfig::default()
        };

        let resolved = builtin_runtime_registry_catalog()
            .resolve_builder_provider("google.providers.yunwu", &config)
            .expect("gemini upstream should resolve a google builder");
        assert_eq!(resolved.catalog_provider, "google");
        assert_eq!(resolved.builder_provider, "google");
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn resolve_builtin_builder_provider_keeps_known_openai_aliases() {
        let resolved = builtin_runtime_registry_catalog()
            .resolve_builder_provider("azure-openai", &ProviderConfig::default())
            .expect("known legacy alias should still resolve");

        assert_eq!(resolved.catalog_provider, "openai-compatible");
        assert_eq!(resolved.builder_provider, "openai-compatible");
    }

    #[cfg(feature = "provider-deepseek")]
    #[test]
    fn context_cache_profile_resolves_from_catalog_behaviors() {
        let profile = builtin_runtime_registry_catalog()
            .resolve_context_cache_profile(
                "deepseek",
                &ProviderConfig {
                    default_model: Some("deepseek-reasoner".to_string()),
                    ..ProviderConfig::default()
                },
                "deepseek-reasoner",
            )
            .expect("deepseek context cache profile should resolve");

        assert!(profile.supports_mode(ContextCacheMode::Passive));
    }

    #[cfg(feature = "provider-minimax")]
    #[test]
    fn context_cache_profile_derives_anthropic_mode_from_bindings() {
        let profile = builtin_runtime_registry_catalog()
            .resolve_context_cache_profile(
                "minimax",
                &ProviderConfig {
                    provider: Some("minimax".to_string()),
                    default_model: Some("MiniMax-M2".to_string()),
                    ..ProviderConfig::default()
                },
                "MiniMax-M2",
            )
            .expect("minimax context cache profile should resolve");

        assert!(profile.supports_mode(ContextCacheMode::Passive));
        assert!(profile.supports_mode(ContextCacheMode::AnthropicCompatible));
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn builtin_provider_supports_file_builder_accepts_generic_openai_family() {
        let config = ProviderConfig {
            provider: Some("openai-compatible".to_string()),
            base_url: Some("https://proxy.example/v1".to_string()),
            ..ProviderConfig::default()
        };

        assert!(
            builtin_runtime_registry_catalog().provider_supports_file_builder("my-proxy", &config)
        );
    }

    #[cfg(feature = "openai")]
    #[test]
    fn builtin_provider_supports_operation_checks_openai_surface() {
        assert!(
            builtin_runtime_registry_catalog().provider_supports_operation(
                "openai",
                "gpt-5",
                OperationKind::RESPONSE
            )
        );
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn rejects_non_openai_compatible_catalog_plugin() {
        let err = builtin_runtime_registry_catalog()
            .resolve_openai_compatible_provider_capability_profile(
                "google-native",
                &ProviderConfig::default(),
            )
            .expect_err("google should not resolve as openai-compatible provider");
        assert!(err.to_string().contains("non-openai-compatible"));
    }
}
