use crate::catalog::{
    AuthMethodKind, CapabilityKind, ProviderAuthHint, ProviderCapabilityResolution, ProviderClass,
    ProviderId, ProviderPluginDescriptor, builtin_registry,
};
use crate::{ProviderResolutionError, Result};

use crate::catalog::OperationKind;
use crate::config::{ProviderApi, ProviderCapabilities, ProviderConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinProviderPreset {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub class: ProviderClass,
    pub default_base_url: Option<&'static str>,
    pub supported_auth: &'static [AuthMethodKind],
    pub auth_hint: Option<ProviderAuthHint>,
    pub model_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinProviderModelCandidate {
    pub provider: &'static str,
    pub provider_display_name: &'static str,
    pub model: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub supported_operations: &'static [OperationKind],
    pub default_base_url: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinProviderCapabilitySummary {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub class: ProviderClass,
    pub default_base_url: Option<&'static str>,
    pub model_count: usize,
    pub capabilities: Vec<CapabilityKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderCapabilityProfile {
    pub provider: &'static str,
    pub provider_display_name: &'static str,
    pub resolution: ProviderCapabilityResolution,
    pub configured_capabilities: Option<ProviderCapabilities>,
    pub effective_capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderConfigSemantics {
    pub provider: &'static str,
    pub provider_display_name: &'static str,
    pub enabled_capabilities: Vec<CapabilityKind>,
}

pub fn builtin_provider_presets() -> Vec<BuiltinProviderPreset> {
    let mut presets: Vec<_> = builtin_registry()
        .plugins()
        .iter()
        .map(preset_from_plugin)
        .collect();
    presets.sort_by(|a, b| a.provider.cmp(b.provider));
    presets
}

pub fn builtin_provider_preset(provider_name_hint: &str) -> Option<BuiltinProviderPreset> {
    let plugin = builtin_registry().plugin_by_hint(provider_name_hint)?;
    Some(preset_from_plugin(plugin))
}

pub fn builtin_provider_capability_summaries() -> Vec<BuiltinProviderCapabilitySummary> {
    let mut summaries: Vec<_> = builtin_registry()
        .plugins()
        .iter()
        .map(summary_from_plugin)
        .collect();
    summaries.sort_by(|a, b| a.provider.cmp(b.provider));
    summaries
}

pub fn builtin_provider_capability_summary(
    provider_name_hint: &str,
) -> Option<BuiltinProviderCapabilitySummary> {
    let plugin = builtin_registry().plugin_by_hint(provider_name_hint)?;
    Some(summary_from_plugin(plugin))
}

pub fn builtin_models_for_provider(provider_name_hint: &str) -> Vec<BuiltinProviderModelCandidate> {
    let Some(plugin) = builtin_registry().plugin_by_hint(provider_name_hint) else {
        return Vec::new();
    };

    let mut models: Vec<_> = plugin
        .models()
        .iter()
        .map(|model| candidate_from_plugin_model(plugin, model))
        .collect();
    models.sort_by(|a, b| a.model.cmp(b.model));
    models
}

pub fn builtin_provider_candidates_for_model(model: &str) -> Vec<BuiltinProviderModelCandidate> {
    if model.trim().is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for plugin in builtin_registry().plugins() {
        for entry in plugin.models() {
            if entry.matches(model) {
                out.push(candidate_from_plugin_model(plugin, entry));
            }
        }
    }
    out.sort_by(|a, b| a.provider.cmp(b.provider).then(a.model.cmp(b.model)));
    out
}

pub fn resolve_provider_config_semantics(
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<ResolvedProviderConfigSemantics> {
    let plugin = resolve_configured_catalog_plugin(provider_name_hint, provider_config)?;
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
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<ResolvedProviderCapabilityProfile> {
    let plugin = resolve_openai_compatible_catalog_plugin(provider_name_hint, provider_config)?;
    let model = provider_config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let resolution = plugin.capability_resolution(model);

    if provider_config.capabilities.is_some() && !resolution.effective_supports(CapabilityKind::LLM)
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

fn preset_from_plugin(plugin: &ProviderPluginDescriptor) -> BuiltinProviderPreset {
    BuiltinProviderPreset {
        provider: plugin.id,
        display_name: plugin.display_name,
        class: plugin.class,
        default_base_url: plugin.default_base_url,
        supported_auth: plugin.supported_auth,
        auth_hint: plugin.auth_hint,
        model_count: plugin.models().len(),
    }
}

fn summary_from_plugin(plugin: &ProviderPluginDescriptor) -> BuiltinProviderCapabilitySummary {
    let runtime_spec = plugin.runtime_spec();
    BuiltinProviderCapabilitySummary {
        provider: plugin.id,
        display_name: plugin.display_name,
        class: plugin.class,
        default_base_url: plugin.default_base_url,
        model_count: plugin.models().len(),
        capabilities: runtime_spec.capabilities.iter().collect(),
    }
}

fn candidate_from_plugin_model(
    plugin: &ProviderPluginDescriptor,
    model: &crate::catalog::ProviderModelDescriptor,
) -> BuiltinProviderModelCandidate {
    BuiltinProviderModelCandidate {
        provider: plugin.id,
        provider_display_name: plugin.display_name,
        model: model.id,
        display_name: model.display_name,
        aliases: model.aliases,
        supported_operations: model.supported_operations,
        default_base_url: plugin.default_base_url,
    }
}

fn resolve_openai_compatible_catalog_plugin(
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<&'static ProviderPluginDescriptor> {
    let plugin = resolve_configured_catalog_plugin(provider_name_hint, provider_config)?;
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

fn resolve_configured_catalog_plugin(
    provider_name_hint: &str,
    provider_config: &ProviderConfig,
) -> Result<&'static ProviderPluginDescriptor> {
    let registry = builtin_registry();
    if let Some(provider) = provider_config
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return registry
            .plugin_by_id(ProviderId::new(provider))
            .or_else(|| registry.plugin_by_hint(provider))
            .ok_or_else(|| {
                ProviderResolutionError::ConfiguredProviderNotFound {
                    provider: provider.to_string(),
                }
                .into()
            });
    }

    registry
        .plugin_for_runtime_request(provider_name_hint, provider_config.runtime_hints())
        .ok_or_else(|| {
            ProviderResolutionError::CatalogProviderNotFound {
                provider: provider_name_hint.trim().to_string(),
            }
            .into()
        })
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

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn builtin_provider_presets_include_generic_openai_compatible() {
        let preset = builtin_provider_preset("openai-compatible")
            .expect("generic openai-compatible preset should exist");
        assert_eq!(preset.provider, "openai-compatible");
        assert_eq!(preset.default_base_url, None);
        assert_eq!(
            preset
                .auth_hint
                .expect("openai-compatible auth hint")
                .env_keys,
            &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"]
        );
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn resolves_unknown_openai_like_provider_to_generic_catalog_profile() {
        let config = ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("custom-model".to_string()),
            ..ProviderConfig::default()
        };

        let profile = resolve_openai_compatible_provider_capability_profile("my-proxy", &config)
            .expect("unknown provider should fall back to generic openai-compatible plugin");
        assert_eq!(profile.provider, "openai-compatible");
        assert!(profile.resolution.effective_supports(CapabilityKind::LLM));
        assert!(profile.effective_capabilities.streaming);
        assert!(!profile.effective_capabilities.prompt_cache);
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn rejects_non_openai_compatible_catalog_plugin() {
        let err = resolve_openai_compatible_provider_capability_profile(
            "google-native",
            &ProviderConfig::default(),
        )
        .expect_err("google should not resolve as openai-compatible provider");
        assert!(err.to_string().contains("non-openai-compatible"));
    }

    #[cfg(feature = "provider-google")]
    #[test]
    fn builtin_provider_preset_preserves_google_query_auth() {
        let preset = builtin_provider_preset("google-native")
            .expect("google preset should match prefixed provider name");
        let auth_hint = preset.auth_hint.expect("google auth hint");
        assert_eq!(preset.provider, "google");
        assert_eq!(
            preset.default_base_url,
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(auth_hint.method, AuthMethodKind::ApiKeyQuery);
        assert_eq!(auth_hint.query_param, Some("key"));
    }

    #[cfg(any(feature = "provider-openai-compatible", feature = "openai-compatible"))]
    #[test]
    fn builtin_provider_capability_summary_reports_llm_capability() {
        let summary = builtin_provider_capability_summary("openai-compatible")
            .expect("generic openai-compatible summary should exist");
        assert_eq!(summary.provider, "openai-compatible");
        assert!(summary.capabilities.contains(&CapabilityKind::LLM));
        assert_eq!(summary.model_count, 0);

        let providers = builtin_provider_capability_summaries()
            .into_iter()
            .map(|entry| entry.provider)
            .collect::<Vec<_>>();
        assert!(providers.contains(&"openai-compatible"));
    }

    #[cfg(feature = "provider-openrouter")]
    #[test]
    fn builtin_provider_candidates_resolve_model_across_plugins() {
        let candidates = builtin_provider_candidates_for_model("google/gemini-2.5-flash-lite");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.provider == "openrouter")
        );
    }
}
