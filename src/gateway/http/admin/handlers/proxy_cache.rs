#[cfg(feature = "gateway-proxy-cache")]
async fn purge_proxy_cache(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<PurgeProxyCacheRequest>,
) -> Result<Json<PurgeProxyCacheResponse>, (StatusCode, Json<ErrorResponse>)> {
    let admin = ensure_admin_write(&state, &headers)?;
    if admin.tenant_id.is_some() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "tenant-scoped admin tokens cannot purge the proxy cache",
        ));
    }

    let Some(cache) = state.proxy_cache.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "proxy cache not enabled",
        ));
    };

    if payload.all {
        {
            let mut cache = cache.lock().await;
            cache.clear();
        }

        let deleted_redis = {
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.redis_store.as_ref() {
                Some(store.clear_proxy_cache().await.map_err(|err| {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage_error",
                        err.to_string(),
                    )
                })?)
            } else {
                None
            }
            #[cfg(not(feature = "gateway-store-redis"))]
            {
                None
            }
        };

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.prometheus_metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("all");
        }

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        append_admin_audit_log(
            &state,
            "admin.proxy_cache.purge",
            serde_json::json!({
                "all": true,
                "cache_key": payload.cache_key.as_deref(),
                "deleted_redis": deleted_redis,
            }),
        )
        .await;

        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: true,
            deleted_redis,
        }));
    }

    let Some(cache_key) = payload
        .cache_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "must set all=true or cache_key",
        ));
    };

    let removed_memory = {
        let mut cache = cache.lock().await;
        cache.remove(cache_key)
    };

    let deleted_redis = {
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.redis_store.as_ref() {
            Some(
                store
                    .delete_proxy_cache_response(cache_key)
                    .await
                    .map_err(|err| {
                        error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "storage_error",
                            err.to_string(),
                        )
                    })?,
            )
        } else {
            None
        }
        #[cfg(not(feature = "gateway-store-redis"))]
        {
            None
        }
    };

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.prometheus_metrics.as_ref() {
        metrics.lock().await.record_proxy_cache_purge("key");
    }

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    append_admin_audit_log(
        &state,
        "admin.proxy_cache.purge",
        serde_json::json!({
            "all": false,
            "cache_key": cache_key,
            "cleared_memory": removed_memory,
            "deleted_redis": deleted_redis,
        }),
    )
    .await;

    Ok(Json(PurgeProxyCacheResponse {
        cleared_memory: removed_memory,
        deleted_redis,
    }))
}
