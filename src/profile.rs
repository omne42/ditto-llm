mod anthropic_model_catalog;
mod catalog_bridge;
mod config_editor;
mod generated_catalogs;
mod google_model_catalog;
mod openai_compatible;
mod openai_model_catalog;
mod openai_models;
mod openai_providers;

mod provider_config {
    pub use crate::config::provider_config::*;
}
mod env {
    pub use crate::config::env::*;
}
mod auth {
    pub use crate::config::auth::*;
}
mod http {
    pub(crate) use crate::config::http::*;
}
mod routing {
    pub use crate::config::routing_policy::*;
}

#[cfg(test)]
mod tests;

pub use anthropic_model_catalog::{
    AnthropicCatalogProvider, AnthropicModelCatalog, AnthropicModelCatalogEntry,
    AnthropicModelPricing, AnthropicModelStatus, anthropic_model_catalog,
    anthropic_model_catalog_entry, anthropic_model_catalog_entry_by_model,
};
pub use auth::{resolve_auth_token, resolve_auth_token_with_default_keys};
pub use catalog_bridge::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    ResolvedProviderCapabilityProfile, ResolvedProviderConfigSemantics,
    builtin_models_for_provider, builtin_provider_candidates_for_model,
    builtin_provider_capability_summaries, builtin_provider_capability_summary,
    builtin_provider_preset, builtin_provider_presets,
    resolve_openai_compatible_provider_capability_profile, resolve_provider_config_semantics,
};
pub use config_editor::{
    ConfigScope, ModelDeleteReport, ModelDeleteRequest, ModelListReport, ModelListRequest,
    ModelShowReport, ModelShowRequest, ModelSummary, ModelUpsertReport, ModelUpsertRequest,
    ProviderAuthType, ProviderDeleteReport, ProviderDeleteRequest, ProviderListReport,
    ProviderListRequest, ProviderNamespace, ProviderShowReport, ProviderShowRequest,
    ProviderSummary, ProviderUpsertReport, ProviderUpsertRequest,
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
    delete_model_config, delete_provider_config, list_model_configs, list_provider_configs,
    show_model_config, show_provider_config, upsert_model_config, upsert_provider_config,
};
pub use env::{Env, parse_dotenv};
pub use google_model_catalog::{
    GoogleCatalogProvider, GoogleModelCatalog, GoogleModelCatalogEntry, GoogleModelVersion,
    GoogleSupportedDataTypes, google_model_catalog, google_model_catalog_entry,
    google_model_catalog_entry_by_model,
};
pub use openai_compatible::OpenAiCompatibleClient;
pub use openai_model_catalog::{
    OpenAiCatalogProvider, OpenAiModalitySupport, OpenAiModelCatalog, OpenAiModelCatalogEntry,
    OpenAiModelRevisions, openai_model_catalog, openai_model_catalog_entry,
};
pub use openai_models::{OpenAiModelsProvider, Provider, list_available_models};
pub use openai_providers::{
    OpenAiProviderFamily, OpenAiProviderQuirks, PromptCacheUsageReporting,
    infer_openai_provider_family, infer_openai_provider_quirks, merge_provider_config,
};
pub use provider_config::{
    ModelConfig, ProviderApi, ProviderAuth, ProviderCapabilities, ProviderConfig,
    ThinkingIntensity, filter_models_whitelist, normalize_string_list, select_model_config,
};
pub use routing::{
    ProviderRoutingConfig, ResolvedRoutingPlan, ResolvedRoutingTarget, RoutingConfigFormat,
    RoutingContext, RoutingOverride, RoutingPhase, RoutingPolicy, RoutingPolicySource,
    RoutingProviderProfile, RoutingStagePolicy, RoutingTarget,
};
