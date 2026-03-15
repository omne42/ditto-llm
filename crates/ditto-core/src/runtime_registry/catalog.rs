use crate::catalog::{CatalogRegistry, ProviderCapabilityResolution, builtin_registry};
use crate::config::ProviderCapabilities;
use crate::contracts::{
    AuthMethodKind, CapabilityKind, OperationKind, ProviderAuthHint, ProviderClass,
};

/// Runtime-registry-facing summary of builtin provider presets derived from `catalog`.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedBuiltinBuilderProvider {
    pub(crate) catalog_provider: &'static str,
    pub(crate) builder_provider: &'static str,
    pub(crate) default_base_url: Option<&'static str>,
}

/// Explicit runtime-registry-owned query view for builtin catalog-derived state.
///
/// The underlying truth still lives in `catalog`, but callers now enter through
/// `runtime_registry` instead of reaching a grab bag of global helper functions.
/// Generated provider catalogs are compiled into the builtin catalog before this
/// view is materialized, so they remain part of the builtin layer rather than a
/// separate precedence tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinRuntimeRegistryCatalog {
    registry: CatalogRegistry,
}

pub fn builtin_runtime_registry_catalog() -> BuiltinRuntimeRegistryCatalog {
    BuiltinRuntimeRegistryCatalog::from_registry(builtin_registry())
}

impl BuiltinRuntimeRegistryCatalog {
    pub(crate) const fn from_registry(registry: CatalogRegistry) -> Self {
        Self { registry }
    }

    pub(super) const fn registry(self) -> CatalogRegistry {
        self.registry
    }
}
