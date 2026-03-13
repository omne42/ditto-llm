use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCacheMode {
    Passive,
    PromptCacheKey,
    AnthropicCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContextCacheProfile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<ContextCacheMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl ContextCacheProfile {
    pub fn supports_caching(&self) -> bool {
        !self.modes.is_empty()
    }

    pub fn supports_mode(&self, mode: ContextCacheMode) -> bool {
        self.modes.contains(&mode)
    }
}

pub trait ContextCacheModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;
    fn context_cache_profile(&self) -> &ContextCacheProfile;
}
