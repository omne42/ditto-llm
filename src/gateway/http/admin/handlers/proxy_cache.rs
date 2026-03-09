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

    let Some(cache) = state.proxy.cache.as_ref() else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_configured",
            "proxy cache not enabled",
        ));
    };

    let selector = payload.selector.into_normalized();

    if payload.all {
        let deleted_memory = {
            let mut cache = cache.lock().await;
            let deleted = cache.len() as u64;
            cache.clear();
            deleted
        };

        let deleted_redis = {
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
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
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            metrics.lock().await.record_proxy_cache_purge("all");
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        append_admin_audit_log(
            &state,
            "admin.proxy_cache.purge",
            serde_json::json!({
                "all": true,
                "selector": selector,
                "deleted_memory": deleted_memory,
                "deleted_redis": deleted_redis,
            }),
        )
        .await;

        return Ok(Json(PurgeProxyCacheResponse {
            cleared_memory: true,
            deleted_memory,
            deleted_redis,
        }));
    }

    if selector.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "must set all=true or at least one of cache_key/scope/method/path/model",
        ));
    }

    let deleted_memory = {
        let mut cache = cache.lock().await;
        cache.purge_matching(&selector)
    };

    let deleted_redis = {
        #[cfg(feature = "gateway-store-redis")]
        if let Some(store) = state.stores.redis.as_ref() {
            Some(
                store
                    .purge_proxy_cache_matching(&selector)
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
    if let Some(metrics) = state.proxy.metrics.as_ref() {
        metrics
            .lock()
            .await
            .record_proxy_cache_purge(selector.kind_label());
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    append_admin_audit_log(
        &state,
        "admin.proxy_cache.purge",
        serde_json::json!({
            "all": false,
            "selector": selector,
            "deleted_memory": deleted_memory,
            "deleted_redis": deleted_redis,
        }),
    )
    .await;

    Ok(Json(PurgeProxyCacheResponse {
        cleared_memory: deleted_memory > 0,
        deleted_memory,
        deleted_redis,
    }))
}
