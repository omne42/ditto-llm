async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn metrics(State(state): State<GatewayHttpState>) -> Json<ObservabilitySnapshot> {
    let gateway = state.gateway.lock().await;
    Json(gateway.observability())
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Deserialize)]
struct PurgeProxyCacheRequest {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    cache_key: Option<String>,
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Serialize)]
struct PurgeProxyCacheResponse {
    cleared_memory: bool,
    deleted_redis: Option<u64>,
}
