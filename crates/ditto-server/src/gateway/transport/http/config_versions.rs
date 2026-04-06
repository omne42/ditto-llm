#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::admin_persistence::append_admin_audit_log;
use super::admin_persistence::apply_control_plane_change;
use super::*;
use omne_integrity_primitives::Sha256Hasher;

#[derive(Clone, Debug, Serialize)]
pub(super) struct ConfigVersionInfo {
    pub(super) version_id: String,
    pub(super) created_at_ms: u64,
    pub(super) reason: String,
    pub(super) virtual_key_count: usize,
    pub(super) virtual_keys_sha256: String,
    pub(super) router_default_backend_count: usize,
    pub(super) router_rule_count: usize,
    pub(super) router_sha256: String,
}

#[derive(Clone, Debug)]
struct ConfigVersionSnapshot {
    info: ConfigVersionInfo,
    virtual_keys: Vec<VirtualKeyConfig>,
    router: RouterConfig,
}

#[derive(Debug)]
pub(super) struct ConfigVersionHistory {
    max_entries: usize,
    next_sequence: u64,
    entries: VecDeque<ConfigVersionSnapshot>,
}

impl ConfigVersionHistory {
    pub(super) fn with_bootstrap(
        virtual_keys: Vec<VirtualKeyConfig>,
        router: RouterConfig,
    ) -> Self {
        let mut history = Self {
            max_entries: 100,
            next_sequence: 1,
            entries: VecDeque::new(),
        };
        let _ = history.push_snapshot(virtual_keys, router, "bootstrap");
        history
    }

    pub(super) fn push_snapshot(
        &mut self,
        virtual_keys: Vec<VirtualKeyConfig>,
        router: RouterConfig,
        reason: impl Into<String>,
    ) -> ConfigVersionInfo {
        let info = ConfigVersionInfo {
            version_id: format!("cfgv-{:020}", self.next_sequence),
            created_at_ms: now_epoch_millis_u64(),
            reason: reason.into(),
            virtual_key_count: virtual_keys.len(),
            virtual_keys_sha256: virtual_keys_sha256(&virtual_keys),
            router_default_backend_count: router.default_backends.len(),
            router_rule_count: router.rules.len(),
            router_sha256: router_sha256(&router),
        };
        self.next_sequence = self.next_sequence.saturating_add(1);

        self.entries.push_back(ConfigVersionSnapshot {
            info: info.clone(),
            virtual_keys,
            router,
        });
        while self.entries.len() > self.max_entries {
            let _ = self.entries.pop_front();
        }

        info
    }

    fn current_info(&self) -> Option<ConfigVersionInfo> {
        self.entries.back().map(|snapshot| snapshot.info.clone())
    }

    fn list_infos_desc(&self) -> Vec<ConfigVersionInfo> {
        self.entries
            .iter()
            .rev()
            .map(|snapshot| snapshot.info.clone())
            .collect()
    }

    fn find_snapshot(&self, version_id: &str) -> Option<ConfigVersionSnapshot> {
        self.entries
            .iter()
            .find(|snapshot| snapshot.info.version_id == version_id)
            .cloned()
    }
}

fn now_epoch_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn virtual_keys_sha256(virtual_keys: &[VirtualKeyConfig]) -> String {
    let payload = serde_json::to_vec(virtual_keys).unwrap_or_default();
    let mut hasher = Sha256Hasher::new();
    hasher.update(b"ditto-gateway-config-version-v1|");
    hasher.update(payload);
    hasher.finalize().to_string()
}

fn router_sha256(router: &RouterConfig) -> String {
    let payload = serde_json::to_vec(router).unwrap_or_default();
    let mut hasher = Sha256Hasher::new();
    hasher.update(b"ditto-gateway-router-version-v1|");
    hasher.update(payload);
    hasher.finalize().to_string()
}

const MAX_CONFIG_VERSIONS_LIMIT: usize = 1_000;

#[derive(Debug, Deserialize)]
pub(super) struct ListConfigVersionsQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct GetConfigVersionQuery {
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfigDiffQuery {
    from_version_id: String,
    to_version_id: String,
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExportConfigQuery {
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    include_tokens: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfigRollbackRequest {
    version_id: String,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfigRouterUpsertRequest {
    router: RouterConfig,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigVersionDetailResponse {
    #[serde(flatten)]
    info: ConfigVersionInfo,
    virtual_keys: Vec<VirtualKeyConfig>,
    router: RouterConfig,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigRollbackResponse {
    dry_run: bool,
    noop: bool,
    rolled_back_to_version_id: String,
    target_version: ConfigVersionInfo,
    current_version: ConfigVersionInfo,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigVersionDiffSummary {
    from_virtual_key_count: usize,
    to_virtual_key_count: usize,
    added: usize,
    removed: usize,
    changed: usize,
    unchanged: usize,
    router_changed: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigVersionDiffEntry {
    id: String,
    before: VirtualKeyConfig,
    after: VirtualKeyConfig,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigVersionDiffResponse {
    from_version: ConfigVersionInfo,
    to_version: ConfigVersionInfo,
    summary: ConfigVersionDiffSummary,
    added: Vec<VirtualKeyConfig>,
    removed: Vec<VirtualKeyConfig>,
    changed: Vec<ConfigVersionDiffEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_before: Option<RouterConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_after: Option<RouterConfig>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfigValidateRequest {
    #[serde(default)]
    virtual_keys: Vec<VirtualKeyConfig>,
    #[serde(default)]
    router: Option<RouterConfig>,
    #[serde(default)]
    expected_virtual_keys_sha256: Option<String>,
    #[serde(default)]
    expected_router_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigValidationIssue {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigValidateResponse {
    valid: bool,
    virtual_key_count: usize,
    computed_virtual_keys_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_default_backend_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_rule_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    computed_router_sha256: Option<String>,
    issues: Vec<ConfigValidationIssue>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigRouterUpsertResponse {
    dry_run: bool,
    noop: bool,
    router_changed: bool,
    target_router_sha256: String,
    previous_version: ConfigVersionInfo,
    current_version: ConfigVersionInfo,
}

fn redact_virtual_key_tokens(keys: &mut [VirtualKeyConfig]) {
    for key in keys {
        key.token = "redacted".to_string();
    }
}

fn redact_diff_change_tokens(changes: &mut [ConfigVersionDiffEntry]) {
    for change in changes {
        change.before.token = "redacted".to_string();
        change.after.token = "redacted".to_string();
    }
}

fn virtual_key_equal(lhs: &VirtualKeyConfig, rhs: &VirtualKeyConfig) -> bool {
    serde_json::to_vec(lhs).ok() == serde_json::to_vec(rhs).ok()
}

fn router_config_equal(lhs: &RouterConfig, rhs: &RouterConfig) -> bool {
    serde_json::to_vec(lhs).ok() == serde_json::to_vec(rhs).ok()
}

fn push_validation_issue(
    issues: &mut Vec<ConfigValidationIssue>,
    code: &'static str,
    message: impl Into<String>,
    path: Option<String>,
) {
    issues.push(ConfigValidationIssue {
        code,
        message: message.into(),
        path,
    });
}

pub(super) async fn get_config_version(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<ConfigVersionInfo>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let history = state.config_versions.lock().await;
    let Some(current) = history.current_info() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };

    Ok(Json(current))
}

pub(super) async fn get_config_version_by_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(version_id): Path<String>,
    Query(query): Query<GetConfigVersionQuery>,
) -> Result<Json<ConfigVersionDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }
    if query.include_tokens {
        ensure_admin_secret_access(&admin)?;
    }

    let version_id = version_id.trim();
    if version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "version_id cannot be empty",
        ));
    }

    let Some(snapshot) = state.config_versions.lock().await.find_snapshot(version_id) else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {version_id}"),
        ));
    };

    let mut virtual_keys = snapshot.virtual_keys;
    if !query.include_tokens {
        redact_virtual_key_tokens(&mut virtual_keys);
    }

    Ok(Json(ConfigVersionDetailResponse {
        info: snapshot.info,
        virtual_keys,
        router: snapshot.router,
    }))
}

pub(super) async fn export_config(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ExportConfigQuery>,
) -> Result<Json<ConfigVersionDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }
    if query.include_tokens {
        ensure_admin_secret_access(&admin)?;
    }

    let snapshot = {
        let history = state.config_versions.lock().await;
        if let Some(version_id) = query
            .version_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            history.find_snapshot(version_id).ok_or_else(|| {
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!("config version not found: {version_id}"),
                )
            })?
        } else {
            let current = history.current_info().ok_or_else(|| {
                error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "config version history is empty",
                )
            })?;
            history
                .find_snapshot(current.version_id.as_str())
                .ok_or_else(|| {
                    error_response(
                        StatusCode::NOT_FOUND,
                        "not_found",
                        "config version history is empty",
                    )
                })?
        }
    };

    let mut virtual_keys = snapshot.virtual_keys;
    if !query.include_tokens {
        redact_virtual_key_tokens(&mut virtual_keys);
    }

    Ok(Json(ConfigVersionDetailResponse {
        info: snapshot.info,
        virtual_keys,
        router: snapshot.router,
    }))
}

pub(super) async fn validate_config_payload(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigValidateRequest>,
) -> Result<Json<ConfigValidateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot validate global config payloads",
        ));
    }

    let computed_virtual_keys_sha256 = virtual_keys_sha256(&payload.virtual_keys);
    let mut issues = Vec::new();
    let mut seen_ids: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut seen_tokens: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

    for (idx, key) in payload.virtual_keys.iter().enumerate() {
        let id = key.id.trim();
        if id.is_empty() {
            push_validation_issue(
                &mut issues,
                "invalid_id",
                "virtual key id cannot be empty",
                Some(format!("/virtual_keys/{idx}/id")),
            );
        } else if let Some(first_idx) = seen_ids.insert(id, idx) {
            push_validation_issue(
                &mut issues,
                "duplicate_id",
                format!("duplicate virtual key id `{id}` (first at index {first_idx})"),
                Some(format!("/virtual_keys/{idx}/id")),
            );
        }

        let token = key.token.trim();
        if token.is_empty() {
            push_validation_issue(
                &mut issues,
                "invalid_token",
                "virtual key token cannot be empty",
                Some(format!("/virtual_keys/{idx}/token")),
            );
        } else if let Some(token_key) =
            crate::gateway::config::normalize_virtual_key_token_key(token)
        {
            if let Some(first_idx) = seen_tokens.insert(token_key, idx) {
                push_validation_issue(
                    &mut issues,
                    "duplicate_token",
                    format!(
                        "duplicate virtual key token at index {idx} (first at index {first_idx})"
                    ),
                    Some(format!("/virtual_keys/{idx}/token")),
                );
            }
        } else {
            push_validation_issue(
                &mut issues,
                "invalid_token",
                "virtual key token cannot be empty",
                Some(format!("/virtual_keys/{idx}/token")),
            );
        }

        if let Err(err) =
            crate::gateway::config::validate_virtual_key_route(key, idx, &backend_names)
        {
            push_validation_issue(
                &mut issues,
                "invalid_route",
                err.to_string(),
                Some(format!("/virtual_keys/{idx}/route")),
            );
        }

        if let Err(err) = crate::gateway::config::validate_virtual_key_guardrails(key, idx) {
            push_validation_issue(
                &mut issues,
                "invalid_guardrails",
                err.to_string(),
                Some(format!("/virtual_keys/{idx}/guardrails")),
            );
        }
    }

    if let Some(expected) = payload
        .expected_virtual_keys_sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && expected != computed_virtual_keys_sha256
    {
        push_validation_issue(
            &mut issues,
            "hash_mismatch",
            format!(
                "virtual_keys_sha256 mismatch: expected={expected}, got={computed_virtual_keys_sha256}"
            ),
            None,
        );
    }

    let mut router_default_backend_count = None;
    let mut router_rule_count = None;
    let mut computed_router_sha256 = None;
    if let Some(router) = payload.router.as_ref() {
        if let Err(err) =
            crate::gateway::config::validate_router_against_backends(router, &backend_names)
        {
            push_validation_issue(
                &mut issues,
                "invalid_router",
                err.to_string(),
                Some("/router".to_string()),
            );
        }

        if let Err(err) = crate::gateway::config::validate_router_guardrails(router) {
            push_validation_issue(
                &mut issues,
                "invalid_guardrails",
                err.to_string(),
                Some("/router".to_string()),
            );
        }

        let computed_router = router_sha256(router);
        if let Some(expected) = payload
            .expected_router_sha256
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && expected != computed_router
        {
            push_validation_issue(
                &mut issues,
                "router_hash_mismatch",
                format!("router_sha256 mismatch: expected={expected}, got={computed_router}"),
                None,
            );
        }
        router_default_backend_count = Some(router.default_backends.len());
        router_rule_count = Some(router.rules.len());
        computed_router_sha256 = Some(computed_router);
    }

    Ok(Json(ConfigValidateResponse {
        valid: issues.is_empty(),
        virtual_key_count: payload.virtual_keys.len(),
        computed_virtual_keys_sha256,
        router_default_backend_count,
        router_rule_count,
        computed_router_sha256,
        issues,
    }))
}

pub(super) async fn upsert_config_router(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigRouterUpsertRequest>,
) -> Result<Json<ConfigRouterUpsertResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot modify global config router",
        ));
    }

    let current_version = {
        let history = state.config_versions.lock().await;
        history.current_info()
    };
    let Some(current_version) = current_version else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };

    let backend_names = state
        .backend_names_snapshot()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let current_router = state.router_config_snapshot();
    crate::gateway::config::validate_router_payload(&payload.router, &backend_names)
        .map_err(map_gateway_error)?;
    let router_changed = !router_config_equal(&current_router, &payload.router);
    let target_router_sha256 = router_sha256(&payload.router);

    if payload.dry_run || !router_changed {
        return Ok(Json(ConfigRouterUpsertResponse {
            dry_run: payload.dry_run,
            noop: true,
            router_changed,
            target_router_sha256,
            previous_version: current_version.clone(),
            current_version,
        }));
    }

    let reason = "admin.config.router.upsert";
    let (_, next_version) = apply_control_plane_change(&state, reason, |gateway| {
        gateway.replace_router_config(payload.router.clone());
        Ok(())
    })
    .await?;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.config.router.upsert",
        serde_json::json!({
            "previous_version_id": current_version.version_id,
            "result_version_id": &next_version.version_id,
            "router_rule_count": next_version.router_rule_count,
            "router_sha256": &next_version.router_sha256,
        }),
    )
    .await?;

    Ok(Json(ConfigRouterUpsertResponse {
        dry_run: false,
        noop: false,
        router_changed: true,
        target_router_sha256,
        previous_version: current_version,
        current_version: next_version,
    }))
}

pub(super) async fn diff_config_versions(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ConfigDiffQuery>,
) -> Result<Json<ConfigVersionDiffResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }
    if query.include_tokens {
        ensure_admin_secret_access(&admin)?;
    }

    let from_version_id = query.from_version_id.trim();
    if from_version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "from_version_id cannot be empty",
        ));
    }

    let to_version_id = query.to_version_id.trim();
    if to_version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "to_version_id cannot be empty",
        ));
    }

    let (from, to) = {
        let history = state.config_versions.lock().await;
        (
            history.find_snapshot(from_version_id),
            history.find_snapshot(to_version_id),
        )
    };
    let Some(from) = from else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {from_version_id}"),
        ));
    };
    let Some(to) = to else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {to_version_id}"),
        ));
    };

    let mut from_by_id: BTreeMap<String, VirtualKeyConfig> = BTreeMap::new();
    for key in from.virtual_keys {
        from_by_id.insert(key.id.clone(), key);
    }

    let mut to_by_id: BTreeMap<String, VirtualKeyConfig> = BTreeMap::new();
    for key in to.virtual_keys {
        to_by_id.insert(key.id.clone(), key);
    }

    let mut key_ids = std::collections::BTreeSet::new();
    key_ids.extend(from_by_id.keys().cloned());
    key_ids.extend(to_by_id.keys().cloned());

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = 0usize;

    for id in key_ids {
        match (from_by_id.get(&id), to_by_id.get(&id)) {
            (Some(before), Some(after)) => {
                if virtual_key_equal(before, after) {
                    unchanged = unchanged.saturating_add(1);
                } else {
                    changed.push(ConfigVersionDiffEntry {
                        id,
                        before: before.clone(),
                        after: after.clone(),
                    });
                }
            }
            (Some(before), None) => removed.push(before.clone()),
            (None, Some(after)) => added.push(after.clone()),
            (None, None) => {}
        }
    }

    if !query.include_tokens {
        redact_virtual_key_tokens(&mut added);
        redact_virtual_key_tokens(&mut removed);
        redact_diff_change_tokens(&mut changed);
    }

    let router_changed = !router_config_equal(&from.router, &to.router);
    let router_before = router_changed.then_some(from.router.clone());
    let router_after = router_changed.then_some(to.router.clone());

    let summary = ConfigVersionDiffSummary {
        from_virtual_key_count: from.info.virtual_key_count,
        to_virtual_key_count: to.info.virtual_key_count,
        added: added.len(),
        removed: removed.len(),
        changed: changed.len(),
        unchanged,
        router_changed,
    };

    Ok(Json(ConfigVersionDiffResponse {
        from_version: from.info,
        to_version: to.info,
        summary,
        added,
        removed,
        changed,
        router_before,
        router_after,
    }))
}

pub(super) async fn list_config_versions(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Query(query): Query<ListConfigVersionsQuery>,
) -> Result<Json<Vec<ConfigVersionInfo>>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_read(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot access global config versions",
        ));
    }

    let mut versions = { state.config_versions.lock().await.list_infos_desc() };

    if query.offset > 0 {
        if query.offset >= versions.len() {
            versions.clear();
        } else {
            versions.drain(0..query.offset);
        }
    }

    if let Some(limit) = query
        .limit
        .map(|value| value.min(MAX_CONFIG_VERSIONS_LIMIT))
        && versions.len() > limit
    {
        versions.truncate(limit);
    }

    Ok(Json(versions))
}

pub(super) async fn rollback_config_version(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigRollbackRequest>,
) -> Result<Json<ConfigRollbackResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot rollback global config",
        ));
    }

    let version_id = payload.version_id.trim();
    if version_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "version_id cannot be empty",
        ));
    }

    let (current, target) = {
        let history = state.config_versions.lock().await;
        (history.current_info(), history.find_snapshot(version_id))
    };
    let Some(current) = current else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "config version history is empty",
        ));
    };
    let Some(target) = target else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("config version not found: {version_id}"),
        ));
    };

    if current.version_id == target.info.version_id {
        return Ok(Json(ConfigRollbackResponse {
            dry_run: payload.dry_run,
            noop: true,
            rolled_back_to_version_id: target.info.version_id.clone(),
            target_version: target.info,
            current_version: current,
        }));
    }

    if payload.dry_run {
        return Ok(Json(ConfigRollbackResponse {
            dry_run: true,
            noop: true,
            rolled_back_to_version_id: version_id.to_string(),
            target_version: target.info,
            current_version: current,
        }));
    }

    let reason = format!("admin.config.rollback:{version_id}");
    let (_, current_version) = apply_control_plane_change(&state, reason.as_str(), |gateway| {
        gateway.replace_virtual_keys(target.virtual_keys.clone());
        gateway.replace_router_config(target.router.clone());
        Ok(())
    })
    .await?;

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.config.rollback",
        serde_json::json!({
            "target_version_id": version_id,
            "result_version_id": &current_version.version_id,
            "virtual_key_count": current_version.virtual_key_count,
            "router_rule_count": current_version.router_rule_count,
            "router_sha256": &current_version.router_sha256,
        }),
    )
    .await?;

    Ok(Json(ConfigRollbackResponse {
        dry_run: false,
        noop: false,
        rolled_back_to_version_id: version_id.to_string(),
        target_version: target.info,
        current_version,
    }))
}
