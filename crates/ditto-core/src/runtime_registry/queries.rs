use super::catalog::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    BuiltinRuntimeRegistryCatalog,
};
use crate::catalog::{ProviderModelDescriptor, ProviderPluginDescriptor};

impl BuiltinRuntimeRegistryCatalog {
    pub fn provider_presets(self) -> Vec<BuiltinProviderPreset> {
        let mut presets: Vec<_> = self
            .registry()
            .plugins()
            .iter()
            .map(preset_from_plugin)
            .collect();
        presets.sort_by(|a, b| a.provider.cmp(b.provider));
        presets
    }

    pub fn provider_preset(self, provider_name_hint: &str) -> Option<BuiltinProviderPreset> {
        let plugin = self.registry().plugin_by_hint(provider_name_hint)?;
        Some(preset_from_plugin(plugin))
    }

    pub fn provider_capability_summaries(self) -> Vec<BuiltinProviderCapabilitySummary> {
        let mut summaries: Vec<_> = self
            .registry()
            .plugins()
            .iter()
            .map(summary_from_plugin)
            .collect();
        summaries.sort_by(|a, b| a.provider.cmp(b.provider));
        summaries
    }

    pub fn provider_capability_summary(
        self,
        provider_name_hint: &str,
    ) -> Option<BuiltinProviderCapabilitySummary> {
        let plugin = self.registry().plugin_by_hint(provider_name_hint)?;
        Some(summary_from_plugin(plugin))
    }

    pub fn models_for_provider(
        self,
        provider_name_hint: &str,
    ) -> Vec<BuiltinProviderModelCandidate> {
        let Some(plugin) = self.registry().plugin_by_hint(provider_name_hint) else {
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

    pub fn provider_candidates_for_model(self, model: &str) -> Vec<BuiltinProviderModelCandidate> {
        if model.trim().is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        for plugin in self.registry().plugins() {
            for entry in plugin.models() {
                if entry.matches(model) {
                    out.push(candidate_from_plugin_model(plugin, entry));
                }
            }
        }
        out.sort_by(|a, b| a.provider.cmp(b.provider).then(a.model.cmp(b.model)));
        out
    }
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
    model: &ProviderModelDescriptor,
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

#[cfg(test)]
mod tests {
    #[cfg(feature = "provider-google")]
    use crate::contracts::AuthMethodKind;
    #[cfg(feature = "provider-openai-compatible")]
    use crate::contracts::CapabilityKind;
    use crate::runtime_registry::builtin_runtime_registry_catalog;

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn builtin_provider_presets_include_generic_openai_compatible() {
        let preset = builtin_runtime_registry_catalog()
            .provider_preset("openai-compatible")
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

    #[cfg(feature = "provider-google")]
    #[test]
    fn builtin_provider_preset_tracks_google_default_header_auth() {
        let preset = builtin_runtime_registry_catalog()
            .provider_preset("google-native")
            .expect("google preset should match prefixed provider name");
        let auth_hint = preset.auth_hint.expect("google auth hint");
        assert_eq!(preset.provider, "google");
        assert_eq!(
            preset.default_base_url,
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert!(
            preset
                .supported_auth
                .contains(&AuthMethodKind::ApiKeyHeader)
        );
        assert!(preset.supported_auth.contains(&AuthMethodKind::ApiKeyQuery));
        assert_eq!(auth_hint.method, AuthMethodKind::ApiKeyHeader);
        assert_eq!(auth_hint.header_name, Some("x-goog-api-key"));
        assert_eq!(auth_hint.env_keys, &["GOOGLE_API_KEY", "GEMINI_API_KEY"]);
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn builtin_provider_capability_summary_reports_llm_capability() {
        let summary = builtin_runtime_registry_catalog()
            .provider_capability_summary("openai-compatible")
            .expect("generic openai-compatible summary should exist");
        assert_eq!(summary.provider, "openai-compatible");
        assert!(summary.capabilities.contains(&CapabilityKind::LLM));
        assert_eq!(summary.model_count, 0);

        let providers = builtin_runtime_registry_catalog()
            .provider_capability_summaries()
            .into_iter()
            .map(|entry| entry.provider)
            .collect::<Vec<_>>();
        assert!(providers.contains(&"openai-compatible"));
    }

    #[cfg(feature = "provider-openrouter")]
    #[test]
    fn builtin_provider_candidates_resolve_model_across_plugins() {
        let candidates = builtin_runtime_registry_catalog()
            .provider_candidates_for_model("google/gemini-2.5-flash-lite");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.provider == "openrouter")
        );
    }
}
