#[cfg(feature = "provider-google")]
mod google;
#[cfg(feature = "provider-openai")]
mod openai;
#[cfg(feature = "provider-openai-compatible")]
mod openai_compatible;

use super::{CatalogRegistry, ProviderPluginDescriptor};

#[cfg(feature = "provider-anthropic")]
use crate::catalog::generated::providers::ANTHROPIC_PLUGIN;
#[cfg(feature = "provider-bailian")]
use crate::catalog::generated::providers::BAILIAN_PLUGIN;
#[cfg(feature = "provider-deepseek")]
use crate::catalog::generated::providers::DEEPSEEK_PLUGIN;
#[cfg(feature = "provider-doubao")]
use crate::catalog::generated::providers::DOUBAO_PLUGIN;
#[cfg(feature = "provider-hunyuan")]
use crate::catalog::generated::providers::HUNYUAN_PLUGIN;
#[cfg(feature = "provider-kimi")]
use crate::catalog::generated::providers::KIMI_PLUGIN;
#[cfg(feature = "provider-minimax")]
use crate::catalog::generated::providers::MINIMAX_PLUGIN;
#[cfg(feature = "provider-openrouter")]
use crate::catalog::generated::providers::OPENROUTER_PLUGIN;
#[cfg(feature = "provider-qianfan")]
use crate::catalog::generated::providers::QIANFAN_PLUGIN;
#[cfg(feature = "provider-xai")]
use crate::catalog::generated::providers::XAI_PLUGIN;
#[cfg(feature = "provider-zhipu")]
use crate::catalog::generated::providers::ZHIPU_PLUGIN;
#[cfg(feature = "provider-google")]
pub use google::BUILTIN_GOOGLE_PLUGIN;

#[cfg(feature = "provider-openai")]
pub use openai::GENERIC_OPENAI_PLUGIN;
#[cfg(feature = "provider-openai-compatible")]
pub use openai_compatible::GENERIC_OPENAI_COMPATIBLE_PLUGIN;

const BUILTIN_PROVIDER_PLUGINS: &[ProviderPluginDescriptor] = &[
    #[cfg(feature = "provider-openai-compatible")]
    GENERIC_OPENAI_COMPATIBLE_PLUGIN,
    #[cfg(feature = "provider-openai")]
    GENERIC_OPENAI_PLUGIN,
    #[cfg(feature = "provider-anthropic")]
    ANTHROPIC_PLUGIN,
    #[cfg(feature = "provider-bailian")]
    BAILIAN_PLUGIN,
    #[cfg(feature = "provider-deepseek")]
    DEEPSEEK_PLUGIN,
    #[cfg(feature = "provider-doubao")]
    DOUBAO_PLUGIN,
    #[cfg(feature = "provider-google")]
    BUILTIN_GOOGLE_PLUGIN,
    #[cfg(feature = "provider-hunyuan")]
    HUNYUAN_PLUGIN,
    #[cfg(feature = "provider-kimi")]
    KIMI_PLUGIN,
    #[cfg(feature = "provider-minimax")]
    MINIMAX_PLUGIN,
    #[cfg(feature = "provider-openrouter")]
    OPENROUTER_PLUGIN,
    #[cfg(feature = "provider-qianfan")]
    QIANFAN_PLUGIN,
    #[cfg(feature = "provider-xai")]
    XAI_PLUGIN,
    #[cfg(feature = "provider-zhipu")]
    ZHIPU_PLUGIN,
];

pub fn builtin_provider_plugins() -> &'static [ProviderPluginDescriptor] {
    BUILTIN_PROVIDER_PLUGINS
}

pub fn builtin_registry() -> CatalogRegistry {
    CatalogRegistry::new(BUILTIN_PROVIDER_PLUGINS)
}
