#[derive(Clone, Copy)]
struct ProxyAttemptParams<'a> {
    state: &'a GatewayHttpState,
    parts: &'a axum::http::request::Parts,
    body: &'a Bytes,
    parsed_json: &'a Option<serde_json::Value>,
    model: &'a Option<String>,
    service_tier: &'a Option<String>,
    request_id: &'a str,
    path_and_query: &'a str,
    now_epoch_seconds: u64,
    charge_tokens: u32,
    max_output_tokens: u32,
    stream_requested: bool,
    strip_authorization: bool,
    use_persistent_budget: bool,
    virtual_key_id: &'a Option<String>,
    budget: &'a Option<super::BudgetConfig>,
    tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    charge_cost_usd_micros: Option<u64>,
    token_budget_reserved: bool,
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    token_budget_reservation_ids: &'a [String],
    cost_budget_reserved: bool,
    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    cost_budget_reservation_ids: &'a [String],
    max_attempts: usize,
    #[cfg(feature = "gateway-routing-advanced")]
    retry_config: &'a super::ProxyRetryConfig,
    #[cfg(feature = "gateway-proxy-cache")]
    proxy_cache_key: &'a Option<String>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    metrics_timer_start: Instant,
}

enum BackendAttemptOutcome {
    Response(axum::response::Response),
    Continue(Option<(StatusCode, Json<OpenAiErrorResponse>)>),
}

include!("multipart_schema.rs");

