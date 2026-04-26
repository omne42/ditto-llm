//! Gateway scope identity helpers.

pub(crate) fn normalize_scope_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|id| !id.is_empty())
}

pub(crate) fn tenant_scope_key(tenant_id: Option<&str>) -> Option<String> {
    normalize_scope_id(tenant_id).map(|tenant_id| format!("tenant:{tenant_id}"))
}

pub(crate) fn project_scope_key(
    tenant_id: Option<&str>,
    project_id: Option<&str>,
) -> Option<String> {
    let project_id = normalize_scope_id(project_id)?;
    if let Some(tenant_id) = normalize_scope_id(tenant_id) {
        return Some(format!("tenant:{tenant_id}:project:{project_id}"));
    }
    Some(format!("project:{project_id}"))
}

pub(crate) fn user_scope_key(tenant_id: Option<&str>, user_id: Option<&str>) -> Option<String> {
    let user_id = normalize_scope_id(user_id)?;
    if let Some(tenant_id) = normalize_scope_id(tenant_id) {
        return Some(format!("tenant:{tenant_id}:user:{user_id}"));
    }
    Some(format!("user:{user_id}"))
}

#[cfg(test)]
mod tests {
    use super::{normalize_scope_id, project_scope_key, tenant_scope_key, user_scope_key};

    #[test]
    fn normalize_scope_id_rejects_absent_or_blank_values() {
        assert_eq!(normalize_scope_id(None), None);
        assert_eq!(normalize_scope_id(Some("")), None);
        assert_eq!(normalize_scope_id(Some("   ")), None);
    }

    #[test]
    fn normalize_scope_id_trims_present_values() {
        assert_eq!(normalize_scope_id(Some(" tenant-a ")), Some("tenant-a"));
    }

    #[test]
    fn tenant_scope_key_requires_tenant() {
        assert_eq!(
            tenant_scope_key(Some(" tenant-a ")),
            Some("tenant:tenant-a".to_owned())
        );
        assert_eq!(tenant_scope_key(Some(" ")), None);
    }

    #[test]
    fn project_scope_key_prefers_tenant_scoped_identity_when_available() {
        assert_eq!(
            project_scope_key(Some(" tenant-a "), Some(" project-a ")),
            Some("tenant:tenant-a:project:project-a".to_owned())
        );
        assert_eq!(
            project_scope_key(Some(" "), Some(" project-a ")),
            Some("project:project-a".to_owned())
        );
        assert_eq!(project_scope_key(Some("tenant-a"), Some(" ")), None);
    }

    #[test]
    fn user_scope_key_prefers_tenant_scoped_identity_when_available() {
        assert_eq!(
            user_scope_key(Some(" tenant-a "), Some(" user-a ")),
            Some("tenant:tenant-a:user:user-a".to_owned())
        );
        assert_eq!(
            user_scope_key(Some(" "), Some(" user-a ")),
            Some("user:user-a".to_owned())
        );
        assert_eq!(user_scope_key(Some("tenant-a"), Some(" ")), None);
    }
}
