async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn metrics(State(state): State<GatewayHttpState>) -> Json<ObservabilitySnapshot> {
    Json(state.observability_snapshot())
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Deserialize)]
struct PurgeProxyCacheRequest {
    #[serde(default)]
    all: bool,
    #[serde(flatten)]
    selector: ProxyCachePurgeSelector,
}

#[cfg(feature = "gateway-proxy-cache")]
#[derive(Debug, Serialize)]
struct PurgeProxyCacheResponse {
    cleared_memory: bool,
    deleted_memory: u64,
    deleted_redis: Option<u64>,
}
