use crate::catalog::{
    AuthMethodKind, OperationKind, ProviderAuthHint, ProviderClass, ProviderPluginDescriptor,
    builtin_registry,
};

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

pub fn builtin_provider_presets() -> Vec<BuiltinProviderPreset> {
    let mut presets: Vec<_> = builtin_registry()
        .plugins()
        .iter()
        .map(|plugin| BuiltinProviderPreset {
            provider: plugin.id,
            display_name: plugin.display_name,
            class: plugin.class,
            default_base_url: plugin.default_base_url,
            supported_auth: plugin.supported_auth,
            auth_hint: plugin.auth_hint,
            model_count: plugin.models().len(),
        })
        .collect();
    presets.sort_by(|a, b| a.provider.cmp(b.provider));
    presets
}

pub fn builtin_provider_preset(provider_name_hint: &str) -> Option<BuiltinProviderPreset> {
    let plugin = best_plugin_match(provider_name_hint)?;
    Some(BuiltinProviderPreset {
        provider: plugin.id,
        display_name: plugin.display_name,
        class: plugin.class,
        default_base_url: plugin.default_base_url,
        supported_auth: plugin.supported_auth,
        auth_hint: plugin.auth_hint,
        model_count: plugin.models().len(),
    })
}

pub fn builtin_models_for_provider(provider_name_hint: &str) -> Vec<BuiltinProviderModelCandidate> {
    let Some(plugin) = best_plugin_match(provider_name_hint) else {
        return Vec::new();
    };

    let mut models: Vec<_> = plugin
        .models()
        .iter()
        .map(|model| BuiltinProviderModelCandidate {
            provider: plugin.id,
            provider_display_name: plugin.display_name,
            model: model.id,
            display_name: model.display_name,
            aliases: model.aliases,
            supported_operations: model.supported_operations,
            default_base_url: plugin.default_base_url,
        })
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
                out.push(BuiltinProviderModelCandidate {
                    provider: plugin.id,
                    provider_display_name: plugin.display_name,
                    model: entry.id,
                    display_name: entry.display_name,
                    aliases: entry.aliases,
                    supported_operations: entry.supported_operations,
                    default_base_url: plugin.default_base_url,
                });
            }
        }
    }
    out.sort_by(|a, b| a.provider.cmp(b.provider).then(a.model.cmp(b.model)));
    out
}

fn best_plugin_match(provider_name_hint: &str) -> Option<&'static ProviderPluginDescriptor> {
    let hint = normalize_token(provider_name_hint);
    if hint.is_empty() {
        return None;
    }

    builtin_registry()
        .plugins()
        .iter()
        .filter_map(|plugin| plugin_match_score(plugin, &hint).map(|score| (plugin, score)))
        .max_by_key(|(_, score)| *score)
        .map(|(plugin, _)| plugin)
}

fn plugin_match_score(plugin: &ProviderPluginDescriptor, normalized_hint: &str) -> Option<usize> {
    let provider = normalize_token(plugin.id);
    let display = normalize_token(plugin.display_name);

    if normalized_hint == provider || normalized_hint == display {
        return Some(100);
    }
    if normalized_hint.starts_with(&provider) || normalized_hint.contains(&provider) {
        return Some(80 + provider.len());
    }
    if normalized_hint.starts_with(&display) || normalized_hint.contains(&display) {
        return Some(60 + display.len());
    }
    None
}

fn normalize_token(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_provider_presets_include_generic_openai() {
        let preset = builtin_provider_preset("openai").expect("generic openai preset should exist");
        assert_eq!(preset.provider, "openai");
        assert_eq!(preset.default_base_url, Some("https://api.openai.com/v1"));
        assert!(preset.model_count > 0);
        assert_eq!(
            preset.auth_hint.expect("openai auth hint").env_keys,
            &["OPENAI_API_KEY"]
        );
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
