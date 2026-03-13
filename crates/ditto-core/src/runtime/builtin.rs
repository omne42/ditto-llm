use crate::catalog::{CatalogRegistry, builtin_registry};
use crate::runtime_registry::BuiltinRuntimeRegistryCatalog;

// RUNTIME-BUILTIN-ASSEMBLY: centralize builtin catalog/runtime-registry access
// inside `runtime` so assembly entrypoints stop scattering global lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BuiltinRuntimeAssembly {
    catalog: CatalogRegistry,
    registry: BuiltinRuntimeRegistryCatalog,
}

pub(super) fn builtin_runtime_assembly() -> BuiltinRuntimeAssembly {
    BuiltinRuntimeAssembly::builtin()
}

impl BuiltinRuntimeAssembly {
    pub(super) fn builtin() -> Self {
        Self::from_catalog(builtin_registry())
    }

    pub(super) const fn from_catalog(catalog: CatalogRegistry) -> Self {
        Self {
            catalog,
            registry: BuiltinRuntimeRegistryCatalog::from_registry(catalog),
        }
    }

    pub(super) const fn catalog(self) -> CatalogRegistry {
        self.catalog
    }

    pub(super) const fn registry(self) -> BuiltinRuntimeRegistryCatalog {
        self.registry
    }
}
