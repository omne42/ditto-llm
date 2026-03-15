//! L0 boundary: runtime_registry.
//!
//! `catalog` owns the static provider/model matrix. `runtime_registry` owns
//! the runtime-facing registry views derived from that matrix: machine-readable
//! provider/model snapshots and config-semantics helpers that higher layers can
//! inspect without re-implementing provider assembly rules.
//!
//! Model/provider truth resolves in this order:
//! 1. explicit user config
//! 2. runtime_registry views
//! 3. compiled builtin catalog
//!
//! Generated provider catalog artifacts fold into the builtin catalog layer; they
//! do not create a separate source-of-truth tier.

mod catalog;
mod queries;
mod semantics;
mod snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTruthSource {
    UserConfig,
    RuntimeRegistry,
    BuiltinCatalog,
}

impl ModelTruthSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserConfig => "user-config",
            Self::RuntimeRegistry => "runtime-registry",
            Self::BuiltinCatalog => "builtin-catalog",
        }
    }

    pub const fn role(self) -> &'static str {
        match self {
            Self::UserConfig => {
                "Dynamic owner for explicit overrides such as configured provider/model selections."
            }
            Self::RuntimeRegistry => {
                "Typed runtime-facing view that higher layers must inspect before re-deriving provider truth."
            }
            Self::BuiltinCatalog => {
                "Compiled fallback metadata layer, including generated provider catalog artifacts."
            }
        }
    }
}

pub const MODEL_TRUTH_PRECEDENCE: [ModelTruthSource; 3] = [
    ModelTruthSource::UserConfig,
    ModelTruthSource::RuntimeRegistry,
    ModelTruthSource::BuiltinCatalog,
];

pub const fn model_truth_precedence() -> &'static [ModelTruthSource] {
    &MODEL_TRUTH_PRECEDENCE
}

pub use catalog::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    BuiltinRuntimeRegistryCatalog, ResolvedProviderCapabilityProfile,
    ResolvedProviderConfigSemantics, builtin_runtime_registry_catalog,
};
pub use snapshot::{
    RuntimeRegistryModelEntry, RuntimeRegistryProviderEntry, RuntimeRegistrySnapshot,
    builtin_runtime_registry,
};
