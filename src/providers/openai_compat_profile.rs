use crate::capabilities::context_cache::{ContextCacheMode, ContextCacheProfile};
use crate::config::ProviderConfig;
use crate::contracts::OperationKind;
use crate::runtime_registry::builtin_runtime_registry_catalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenAiProviderFamily {
    OpenAi,
    OpenRouter,
    DeepSeek,
    Kimi,
    MiniMax,
    Qwen,
    Glm,
    Doubao,
    LiteLlm,
    GenericOpenAiCompatible,
}

impl OpenAiProviderFamily {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::DeepSeek => "deepseek",
            Self::Kimi => "kimi",
            Self::MiniMax => "minimax",
            Self::Qwen => "qwen",
            Self::Glm => "glm",
            Self::Doubao => "doubao",
            Self::LiteLlm => "litellm",
            Self::GenericOpenAiCompatible => "openai-compatible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptCacheUsageReporting {
    Reliable,
    BestEffort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OpenAiCompatibleModelBehavior {
    pub(crate) assistant_tool_call_requires_reasoning_content: bool,
    pub(crate) tool_choice_required_supported: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThoughtSignaturePassthroughPolicy {
    Never,
    GeminiModels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiCompatibilityProfile {
    family: OpenAiProviderFamily,
    prompt_cache_usage_reporting: PromptCacheUsageReporting,
    catalog_provider: Option<&'static str>,
    thought_signature_passthrough_policy: ThoughtSignaturePassthroughPolicy,
}

impl OpenAiCompatibilityProfile {
    pub(crate) fn resolve(provider: &str, base_url: &str, config: Option<&ProviderConfig>) -> Self {
        let explicit_family = config.and_then(explicit_family_override);
        let family =
            explicit_family.unwrap_or_else(|| infer_openai_provider_family(provider, base_url));
        let catalog_provider = config
            .and_then(|provider_config| {
                builtin_runtime_registry_catalog()
                    .resolve_builder_provider(provider, provider_config)
                    .map(|resolved| resolved.catalog_provider)
            })
            .or_else(|| default_catalog_provider_for_family(family));

        let prompt_cache_usage_reporting = match family {
            OpenAiProviderFamily::Qwen
            | OpenAiProviderFamily::Doubao
            | OpenAiProviderFamily::OpenRouter => PromptCacheUsageReporting::BestEffort,
            _ => PromptCacheUsageReporting::Reliable,
        };

        let thought_signature_passthrough_policy = match family {
            OpenAiProviderFamily::LiteLlm => ThoughtSignaturePassthroughPolicy::GeminiModels,
            _ => ThoughtSignaturePassthroughPolicy::Never,
        };

        Self {
            family,
            prompt_cache_usage_reporting,
            catalog_provider,
            thought_signature_passthrough_policy,
        }
    }

    pub(crate) const fn family(&self) -> OpenAiProviderFamily {
        self.family
    }

    #[cfg(test)]
    pub(crate) fn prompt_cache_usage_may_be_missing(&self) -> bool {
        matches!(
            self.prompt_cache_usage_reporting,
            PromptCacheUsageReporting::BestEffort
        )
    }

    pub(crate) const fn default_allow_prompt_cache_key(&self) -> bool {
        false
    }

    pub(crate) fn default_assistant_tool_call_requires_reasoning_content(&self) -> bool {
        matches!(self.family, OpenAiProviderFamily::Kimi)
    }

    pub(crate) fn should_send_tool_call_thought_signature(&self, model: &str) -> bool {
        match self.thought_signature_passthrough_policy {
            ThoughtSignaturePassthroughPolicy::Never => false,
            ThoughtSignaturePassthroughPolicy::GeminiModels => {
                model.to_ascii_lowercase().contains("gemini")
            }
        }
    }

    pub(crate) fn model_behavior(&self, model: &str) -> OpenAiCompatibleModelBehavior {
        let mut behavior = OpenAiCompatibleModelBehavior {
            assistant_tool_call_requires_reasoning_content: self
                .default_assistant_tool_call_requires_reasoning_content(),
            tool_choice_required_supported: None,
        };

        let model = model.trim();
        let Some(catalog_provider) = self.catalog_provider else {
            return behavior;
        };
        if model.is_empty() {
            return behavior;
        }

        let registry = builtin_runtime_registry_catalog();
        behavior.assistant_tool_call_requires_reasoning_content |= registry
            .provider_requires_reasoning_content_followup(
                catalog_provider,
                model,
                OperationKind::CHAT_COMPLETION,
            );
        behavior.tool_choice_required_supported = registry.provider_required_tool_choice_support(
            catalog_provider,
            model,
            OperationKind::CHAT_COMPLETION,
        );
        behavior
    }

    pub(crate) fn context_cache_profile(&self, model: Option<&str>) -> ContextCacheProfile {
        if let (Some(catalog_provider), Some(model)) = (self.catalog_provider, model) {
            if let Some(profile) = builtin_runtime_registry_catalog()
                .resolve_catalog_context_cache_profile(catalog_provider, model)
            {
                return profile;
            }
        }

        match self.family {
            OpenAiProviderFamily::DeepSeek => ContextCacheProfile {
                modes: vec![ContextCacheMode::Passive],
                notes: Some(
                    "DeepSeek context caching is passive and applied automatically to repeated prefixes."
                        .to_string(),
                ),
            },
            OpenAiProviderFamily::MiniMax => ContextCacheProfile {
                modes: vec![
                    ContextCacheMode::Passive,
                    ContextCacheMode::AnthropicCompatible,
                ],
                notes: Some(
                    "MiniMax exposes passive prompt caching plus Anthropic-compatible active cache semantics."
                        .to_string(),
                ),
            },
            _ => ContextCacheProfile::default(),
        }
    }
}

fn explicit_family_override(config: &ProviderConfig) -> Option<OpenAiProviderFamily> {
    config
        .openai_compatible
        .as_ref()
        .and_then(|explicit| explicit.family.as_deref())
        .and_then(parse_openai_provider_family)
}

fn default_catalog_provider_for_family(family: OpenAiProviderFamily) -> Option<&'static str> {
    match family {
        OpenAiProviderFamily::OpenAi => Some("openai"),
        OpenAiProviderFamily::OpenRouter => Some("openrouter"),
        OpenAiProviderFamily::DeepSeek => Some("deepseek"),
        OpenAiProviderFamily::Kimi => Some("kimi"),
        OpenAiProviderFamily::MiniMax => Some("minimax"),
        OpenAiProviderFamily::Qwen => Some("qwen"),
        OpenAiProviderFamily::Glm => Some("glm"),
        OpenAiProviderFamily::Doubao => Some("doubao"),
        OpenAiProviderFamily::LiteLlm | OpenAiProviderFamily::GenericOpenAiCompatible => {
            Some("openai-compatible")
        }
    }
}

fn parse_openai_provider_family(value: &str) -> Option<OpenAiProviderFamily> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(OpenAiProviderFamily::OpenAi),
        "openrouter" => Some(OpenAiProviderFamily::OpenRouter),
        "deepseek" => Some(OpenAiProviderFamily::DeepSeek),
        "kimi" | "moonshot" | "moonshotai" => Some(OpenAiProviderFamily::Kimi),
        "minimax" => Some(OpenAiProviderFamily::MiniMax),
        "qwen" => Some(OpenAiProviderFamily::Qwen),
        "glm" | "zhipu" => Some(OpenAiProviderFamily::Glm),
        "doubao" | "ark" => Some(OpenAiProviderFamily::Doubao),
        "litellm" => Some(OpenAiProviderFamily::LiteLlm),
        "openai-compatible" | "generic" => Some(OpenAiProviderFamily::GenericOpenAiCompatible),
        _ => None,
    }
}

fn infer_openai_provider_family(provider: &str, base_url: &str) -> OpenAiProviderFamily {
    let provider = provider.to_ascii_lowercase();
    let base_url = base_url.to_ascii_lowercase();

    if provider.contains("litellm") || base_url.contains("litellm") {
        return OpenAiProviderFamily::LiteLlm;
    }
    if provider.contains("openrouter") || base_url.contains("openrouter.ai") {
        return OpenAiProviderFamily::OpenRouter;
    }
    if provider.contains("deepseek") || base_url.contains("deepseek") {
        return OpenAiProviderFamily::DeepSeek;
    }
    if provider.contains("moonshot")
        || provider.contains("kimi")
        || base_url.contains("moonshot")
        || base_url.contains("kimi")
    {
        return OpenAiProviderFamily::Kimi;
    }
    if provider.contains("minimax") || base_url.contains("minimax") {
        return OpenAiProviderFamily::MiniMax;
    }
    if provider.contains("qwen") || base_url.contains("dashscope") || base_url.contains("aliyuncs")
    {
        return OpenAiProviderFamily::Qwen;
    }
    if provider.contains("glm") || base_url.contains("bigmodel") || base_url.contains("zhipu") {
        return OpenAiProviderFamily::Glm;
    }
    if provider.contains("doubao")
        || provider.contains("ark")
        || base_url.contains("volces")
        || base_url.contains("volcengine")
        || base_url.contains("/api/v3")
    {
        return OpenAiProviderFamily::Doubao;
    }
    if provider.contains("openai") || base_url.contains("api.openai.com") {
        return OpenAiProviderFamily::OpenAi;
    }

    OpenAiProviderFamily::GenericOpenAiCompatible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_openai_profile_marks_qwen_as_best_effort_cache_usage() {
        let profile = OpenAiCompatibilityProfile::resolve(
            "qwen-direct",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            None,
        );
        assert_eq!(profile.family(), OpenAiProviderFamily::Qwen);
        assert!(profile.prompt_cache_usage_may_be_missing());
    }

    #[test]
    fn infer_openai_profile_marks_deepseek_as_reliable_cache_usage() {
        let profile = OpenAiCompatibilityProfile::resolve(
            "deepseek-direct",
            "https://api.deepseek.com/v1",
            None,
        );
        assert_eq!(profile.family(), OpenAiProviderFamily::DeepSeek);
        assert!(!profile.prompt_cache_usage_may_be_missing());
    }

    #[test]
    fn infer_openai_profile_marks_openrouter_as_best_effort_cache_usage() {
        let profile = OpenAiCompatibilityProfile::resolve(
            "openrouter-direct",
            "https://openrouter.ai/api/v1",
            None,
        );
        assert_eq!(profile.family(), OpenAiProviderFamily::OpenRouter);
        assert!(profile.prompt_cache_usage_may_be_missing());
    }
}
