#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Deserialize)]
struct LedgerQuery {
    #[serde(default)]
    key_prefix: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_audit_limit() -> usize {
    100
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_audit_export_limit() -> usize {
    1000
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
const MAX_ADMIN_LEDGER_LIMIT: usize = 10_000;

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn apply_admin_list_window<T>(items: &mut Vec<T>, offset: usize, limit: Option<usize>, max: usize) {
    if offset > 0 {
        if offset >= items.len() {
            items.clear();
        } else {
            items.drain(0..offset);
        }
    }

    if let Some(limit) = limit.map(|limit| limit.min(max)) {
        if items.len() > limit {
            items.truncate(limit);
        }
    }
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn tenant_allowed_scopes(keys: &[VirtualKeyConfig], tenant_id: &str) -> std::collections::HashSet<String> {
    let tenant_id = tenant_id.trim();
    let mut scopes = std::collections::HashSet::<String>::new();
    if !tenant_id.is_empty() {
        scopes.insert(format!("tenant:{tenant_id}"));
    }

    for key in keys {
        if key.tenant_id.as_deref() != Some(tenant_id) {
            continue;
        }
        scopes.insert(key.id.clone());

        if let Some(project_id) = key
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("project:{project_id}"));
        }

        if let Some(user_id) = key
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scopes.insert(format!("user:{user_id}"));
        }
    }

    scopes
}
