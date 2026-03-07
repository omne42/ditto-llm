mod openai;

use super::{CatalogRegistry, ProviderPluginDescriptor};

#[cfg(feature = "provider-anthropic")]
use crate::catalog::generated::ANTHROPIC_PLUGIN;
#[cfg(feature = "provider-bailian")]
use crate::catalog::generated::BAILIAN_PLUGIN;
#[cfg(feature = "provider-deepseek")]
use crate::catalog::generated::DEEPSEEK_PLUGIN;
#[cfg(feature = "provider-doubao")]
use crate::catalog::generated::DOUBAO_PLUGIN;
#[cfg(feature = "provider-google")]
use crate::catalog::generated::GOOGLE_PLUGIN;
#[cfg(feature = "provider-hunyuan")]
use crate::catalog::generated::HUNYUAN_PLUGIN;
#[cfg(feature = "provider-kimi")]
use crate::catalog::generated::KIMI_PLUGIN;
#[cfg(feature = "provider-minimax")]
use crate::catalog::generated::MINIMAX_PLUGIN;
#[cfg(feature = "provider-openrouter")]
use crate::catalog::generated::OPENROUTER_PLUGIN;
#[cfg(feature = "provider-qianfan")]
use crate::catalog::generated::QIANFAN_PLUGIN;
#[cfg(feature = "provider-xai")]
use crate::catalog::generated::XAI_PLUGIN;
#[cfg(feature = "provider-zhipu")]
use crate::catalog::generated::ZHIPU_PLUGIN;

#[cfg(feature = "openai")]
pub use openai::GENERIC_OPENAI_PLUGIN;

const BUILTIN_PROVIDER_PLUGINS: &[ProviderPluginDescriptor] = &[
    #[cfg(feature = "openai")]
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
    GOOGLE_PLUGIN,
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
