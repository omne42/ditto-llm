use crate::catalog::CatalogRegistry;
use crate::config::ProviderConfig;
use crate::{OperationKind, Result, RuntimeRoute, RuntimeRouteRequest, builtin_registry};

#[derive(Debug, Clone, Copy)]
pub struct RuntimeCatalogResolver {
    registry: CatalogRegistry,
}

impl Default for RuntimeCatalogResolver {
    fn default() -> Self {
        Self::builtin()
    }
}

impl RuntimeCatalogResolver {
    pub const fn new(registry: CatalogRegistry) -> Self {
        Self { registry }
    }

    pub fn builtin() -> Self {
        Self::new(crate::builtin_registry())
    }

    pub fn registry(self) -> CatalogRegistry {
        self.registry
    }

    pub fn resolve_route(self, request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
        self.registry.resolve_runtime_route(request)
    }

    pub fn resolve_for_provider(
        self,
        provider: &str,
        model: Option<&str>,
        operation: OperationKind,
        provider_config: Option<&ProviderConfig>,
    ) -> Result<RuntimeRoute> {
        let mut request = RuntimeRouteRequest::new(provider, model, operation);
        if let Some(provider_config) = provider_config {
            request = request.with_provider_config(provider_config);
        }
        self.resolve_route(request)
    }
}

pub fn resolve_builtin_runtime_route(request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
    builtin_registry().resolve_runtime_route(request)
}
