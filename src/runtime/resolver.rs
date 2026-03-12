use super::builtin::builtin_runtime_assembly;
use super::route::resolve_runtime_route;
use crate::contracts::{RuntimeRoute, RuntimeRouteRequest};
use crate::foundation::error::Result;

pub fn resolve_builtin_runtime_route(request: RuntimeRouteRequest<'_>) -> Result<RuntimeRoute> {
    let runtime = builtin_runtime_assembly();
    resolve_runtime_route(&runtime.catalog(), request)
}
