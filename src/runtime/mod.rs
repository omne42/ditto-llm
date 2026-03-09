//! Runtime assembly facade.
//!
//! `catalog` owns static provider/model metadata. `config` owns dynamic user input.
//! `runtime` is the join layer that resolves a concrete invocation route from both.
//! This keeps `gateway` from becoming the only place where runtime assembly exists.

#[doc(hidden)]
pub mod model_builders;
mod provider_catalog;
mod resolver;

pub use provider_catalog::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    ResolvedProviderCapabilityProfile, ResolvedProviderConfigSemantics,
    builtin_models_for_provider, builtin_provider_candidates_for_model,
    builtin_provider_capability_summaries, builtin_provider_capability_summary,
    builtin_provider_preset, builtin_provider_presets,
    resolve_openai_compatible_provider_capability_profile, resolve_provider_config_semantics,
};
pub use resolver::{RuntimeCatalogResolver, resolve_builtin_runtime_route};
