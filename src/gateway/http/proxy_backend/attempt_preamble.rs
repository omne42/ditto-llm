    let backend_name = backend_name.to_string();
    let state = params.state;
    let parts = params.parts;
    let body = params.body;
    let parsed_json = params.parsed_json;
    let model = params.model;
    let service_tier = params.service_tier;
    let request_id = params.request_id.to_string();
    let path_and_query = params.path_and_query;
    let _now_epoch_seconds = params.now_epoch_seconds;
    let charge_tokens = params.charge_tokens;
    let _max_output_tokens = params.max_output_tokens;
    let _stream_requested = params.stream_requested;
    let strip_authorization = params.strip_authorization;
    let use_persistent_budget = params.use_persistent_budget;
    let virtual_key_id = params.virtual_key_id;
    let budget = params.budget;
    let tenant_budget_scope = params.tenant_budget_scope;
    let project_budget_scope = params.project_budget_scope;
    let user_budget_scope = params.user_budget_scope;
    let charge_cost_usd_micros = params.charge_cost_usd_micros;
    let _token_budget_reserved = params.token_budget_reserved;
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    let _cost_budget_reserved = params.cost_budget_reserved;
    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    let max_attempts = params.max_attempts;
    #[cfg(feature = "gateway-routing-advanced")]
    let retry_config = params.retry_config;

    #[cfg(feature = "gateway-proxy-cache")]
    let proxy_cache_key = params.proxy_cache_key;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = idx;

    #[cfg(not(any(
        feature = "gateway-costing",
        feature = "gateway-store-sqlite",
        feature = "gateway-store-redis"
    )))]
    let _ = service_tier;

    #[cfg(not(feature = "gateway-costing"))]
    let _ = use_persistent_budget;

    #[cfg(not(feature = "gateway-routing-advanced"))]
    let _ = max_attempts;

