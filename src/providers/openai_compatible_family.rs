use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum OpenAiProviderFamily {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "kimi")]
    Kimi,
    #[serde(rename = "minimax")]
    MiniMax,
    #[serde(rename = "qwen")]
    Qwen,
    #[serde(rename = "glm")]
    Glm,
    #[serde(rename = "doubao")]
    Doubao,
    #[serde(rename = "openai-compatible")]
    GenericOpenAiCompatible,
}

impl OpenAiProviderFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::DeepSeek => "deepseek",
            Self::Kimi => "kimi",
            Self::MiniMax => "minimax",
            Self::Qwen => "qwen",
            Self::Glm => "glm",
            Self::Doubao => "doubao",
            Self::GenericOpenAiCompatible => "openai-compatible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheUsageReporting {
    Reliable,
    BestEffort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiProviderQuirks {
    pub family: OpenAiProviderFamily,
    pub prompt_cache_usage_reporting: PromptCacheUsageReporting,
}

impl OpenAiProviderQuirks {
    const fn reliable(family: OpenAiProviderFamily) -> Self {
        Self {
            family,
            prompt_cache_usage_reporting: PromptCacheUsageReporting::Reliable,
        }
    }

    const fn best_effort(family: OpenAiProviderFamily) -> Self {
        Self {
            family,
            prompt_cache_usage_reporting: PromptCacheUsageReporting::BestEffort,
        }
    }

    pub fn prompt_cache_usage_may_be_missing(self) -> bool {
        matches!(
            self.prompt_cache_usage_reporting,
            PromptCacheUsageReporting::BestEffort
        )
    }
}

pub fn infer_openai_provider_quirks(provider: &str, base_url: &str) -> OpenAiProviderQuirks {
    let family = infer_openai_provider_family(provider, base_url);
    match family {
        OpenAiProviderFamily::Qwen => OpenAiProviderQuirks::best_effort(family),
        OpenAiProviderFamily::Doubao => OpenAiProviderQuirks::best_effort(family),
        OpenAiProviderFamily::OpenRouter => OpenAiProviderQuirks::best_effort(family),
        _ => OpenAiProviderQuirks::reliable(family),
    }
}

pub fn infer_openai_provider_family(provider: &str, base_url: &str) -> OpenAiProviderFamily {
    let provider = provider.to_ascii_lowercase();
    let base_url = base_url.to_ascii_lowercase();

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
