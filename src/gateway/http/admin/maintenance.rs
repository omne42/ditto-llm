#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Deserialize)]
struct ReapReservationsRequest {
    #[serde(default = "default_reap_reservations_older_than_secs")]
    older_than_secs: u64,
    #[serde(default = "default_reap_reservations_limit")]
    limit: usize,
    #[serde(default)]
    dry_run: bool,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_reap_reservations_older_than_secs() -> u64 {
    24 * 60 * 60
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn default_reap_reservations_limit() -> usize {
    1000
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Serialize)]
struct ReapReservationsCounts {
    scanned: u64,
    reaped: u64,
    released: u64,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
#[derive(Debug, Serialize)]
struct ReapReservationsResponse {
    store: &'static str,
    dry_run: bool,
    cutoff_ts_ms: u64,
    budget: ReapReservationsCounts,
    cost: ReapReservationsCounts,
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
fn now_millis_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn reap_reservations(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<ReapReservationsRequest>,
) -> Result<Json<ReapReservationsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot reap reservations",
        ));
    }

    let now_ts_ms = now_millis_u64();
    let cutoff_ts_ms = now_ts_ms.saturating_sub(payload.older_than_secs.saturating_mul(1000));
    let limit = payload.limit.clamp(1, 100_000);
    let dry_run = payload.dry_run;

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let (budget_scanned, budget_reaped, budget_released) = store
            .reap_stale_budget_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;
        let (cost_scanned, cost_reaped, cost_released) = store
            .reap_stale_cost_reservations(cutoff_ts_ms, limit, dry_run)
            .await
            .map_err(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    err.to_string(),
                )
            })?;

        return Ok(Json(ReapReservationsResponse {
            store: "redis",
            dry_run,
            cutoff_ts_ms,
            budget: ReapReservationsCounts {
                scanned: budget_scanned,
                reaped: budget_reaped,
                released: budget_released,
            },
            cost: ReapReservationsCounts {
                scanned: cost_scanned,
                reaped: cost_reaped,
                released: cost_released,
            },
        }));
    }

    #[cfg(feature = "gateway-store-sqlite")]
    if state.sqlite_store.as_ref().is_some() {
        return Err(error_response(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "reservation reaper is not implemented for sqlite store yet; use redis store for distributed budgets",
        ));
    }

    Err(error_response(
        StatusCode::BAD_REQUEST,
        "not_configured",
        "store not configured",
    ))
}
