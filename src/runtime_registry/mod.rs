//! L0 boundary: runtime_registry.
//!
//! `catalog` owns the static provider/model matrix. `runtime_registry` owns
//! the runtime-facing registry views derived from that matrix: machine-readable
//! provider/model snapshots and config-semantics helpers that higher layers can
//! inspect without re-implementing provider assembly rules.

mod catalog;
mod queries;
mod semantics;
mod snapshot;

pub use catalog::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    BuiltinRuntimeRegistryCatalog, ResolvedProviderCapabilityProfile,
    ResolvedProviderConfigSemantics, builtin_runtime_registry_catalog,
};
pub use snapshot::{
    RuntimeRegistryModelEntry, RuntimeRegistryProviderEntry, RuntimeRegistrySnapshot,
    builtin_runtime_registry,
};
