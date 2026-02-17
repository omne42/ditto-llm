#[cfg(all(feature = "gateway", feature = "sdk"))]
pub(crate) fn attach_devtools(
    state: ditto_llm::gateway::GatewayHttpState,
    devtools_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(path) = devtools_path else {
        return Ok(state);
    };
    Ok(state.with_devtools_logger(ditto_llm::sdk::devtools::DevtoolsLogger::new(path)))
}

#[cfg(all(feature = "gateway", not(feature = "sdk")))]
pub(crate) fn attach_devtools(
    state: ditto_llm::gateway::GatewayHttpState,
    devtools_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if devtools_path.is_some() {
        return Err(
            "devtools requires `--features gateway-devtools` (or `sdk` alongside `gateway`)".into(),
        );
    }
    Ok(state)
}

#[cfg(feature = "gateway")]
const DEFAULT_PROXY_MAX_IN_FLIGHT: usize = 256;

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_backpressure(
    state: ditto_llm::gateway::GatewayHttpState,
    max_in_flight: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let max = max_in_flight.unwrap_or(DEFAULT_PROXY_MAX_IN_FLIGHT);
    if max == 0 {
        return Err("--proxy-max-in-flight must be > 0".into());
    }
    Ok(state.with_proxy_max_in_flight(max))
}

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_max_body_bytes(
    state: ditto_llm::gateway::GatewayHttpState,
    max_body_bytes: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(max) = max_body_bytes else {
        return Ok(state);
    };
    if max == 0 {
        return Err("--proxy-max-body-bytes must be > 0".into());
    }
    Ok(state.with_proxy_max_body_bytes(max))
}

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_usage_max_body_bytes(
    state: ditto_llm::gateway::GatewayHttpState,
    max_body_bytes: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(max) = max_body_bytes else {
        return Ok(state);
    };
    Ok(state.with_proxy_usage_max_body_bytes(max))
}

#[cfg(all(feature = "gateway", feature = "gateway-proxy-cache"))]
pub(crate) fn attach_proxy_cache(
    state: ditto_llm::gateway::GatewayHttpState,
    enabled: bool,
    ttl_seconds: Option<u64>,
    max_entries: Option<usize>,
    max_body_bytes: Option<usize>,
    max_total_body_bytes: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(state);
    }

    let mut config = ditto_llm::gateway::ProxyCacheConfig::default();
    config.ttl_seconds = ttl_seconds.unwrap_or(config.ttl_seconds).max(1);
    config.max_entries = max_entries.unwrap_or(config.max_entries).max(1);
    if let Some(max_body_bytes) = max_body_bytes {
        config.max_body_bytes = max_body_bytes.max(1);
    }
    if let Some(max_total_body_bytes) = max_total_body_bytes {
        config.max_total_body_bytes = max_total_body_bytes.max(1);
    }
    Ok(state.with_proxy_cache(config))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-proxy-cache")))]
pub(crate) fn attach_proxy_cache(
    state: ditto_llm::gateway::GatewayHttpState,
    enabled: bool,
    _ttl_seconds: Option<u64>,
    _max_entries: Option<usize>,
    _max_body_bytes: Option<usize>,
    _max_total_body_bytes: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if enabled {
        return Err("proxy cache requires `--features gateway-proxy-cache`".into());
    }
    Ok(state)
}

#[cfg(feature = "gateway")]
#[derive(Default)]
pub(crate) struct ProxyRoutingCliOptions {
    pub(crate) retry_enabled: bool,
    pub(crate) retry_status_codes: Option<Vec<u16>>,
    pub(crate) retry_max_attempts: Option<usize>,
    pub(crate) circuit_breaker_enabled: bool,
    pub(crate) cb_failure_threshold: Option<u32>,
    pub(crate) cb_cooldown_secs: Option<u64>,
    pub(crate) health_checks_enabled: bool,
    pub(crate) health_check_path: Option<String>,
    pub(crate) health_check_interval_secs: Option<u64>,
    pub(crate) health_check_timeout_secs: Option<u64>,
}

#[cfg(feature = "gateway")]
impl ProxyRoutingCliOptions {
    fn any_set(&self) -> bool {
        self.retry_enabled
            || self.retry_status_codes.is_some()
            || self.retry_max_attempts.is_some()
            || self.circuit_breaker_enabled
            || self.cb_failure_threshold.is_some()
            || self.cb_cooldown_secs.is_some()
            || self.health_checks_enabled
            || self.health_check_path.is_some()
            || self.health_check_interval_secs.is_some()
            || self.health_check_timeout_secs.is_some()
    }
}

#[cfg(all(feature = "gateway", feature = "gateway-routing-advanced"))]
pub(crate) fn attach_proxy_routing(
    state: ditto_llm::gateway::GatewayHttpState,
    opts: ProxyRoutingCliOptions,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !opts.any_set() {
        return Ok(state);
    }

    let mut config = ditto_llm::gateway::ProxyRoutingConfig::default();
    if opts.retry_enabled {
        config.retry.enabled = true;
    }
    if let Some(codes) = opts.retry_status_codes {
        config.retry.retry_status_codes = codes;
    }
    config.retry.max_attempts = opts.retry_max_attempts.filter(|v| *v > 0);

    if opts.circuit_breaker_enabled {
        config.circuit_breaker.enabled = true;
    }
    if let Some(threshold) = opts.cb_failure_threshold {
        config.circuit_breaker.failure_threshold = threshold.max(1);
    }
    if let Some(cooldown) = opts.cb_cooldown_secs {
        config.circuit_breaker.cooldown_seconds = cooldown;
    }

    if opts.health_checks_enabled {
        config.health_check.enabled = true;
    }
    if let Some(path) = opts.health_check_path {
        if path.trim().is_empty() {
            return Err("health check path must be non-empty".into());
        }
        config.health_check.path = path;
    }
    if let Some(interval) = opts.health_check_interval_secs {
        config.health_check.interval_seconds = interval.max(1);
    }
    if let Some(timeout) = opts.health_check_timeout_secs {
        config.health_check.timeout_seconds = timeout.max(1);
    }

    Ok(state.with_proxy_routing(config))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-routing-advanced")))]
pub(crate) fn attach_proxy_routing(
    state: ditto_llm::gateway::GatewayHttpState,
    opts: ProxyRoutingCliOptions,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if opts.any_set() {
        return Err("proxy routing requires `--features gateway-routing-advanced`".into());
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-costing"))]
pub(crate) fn attach_pricing_table(
    state: ditto_llm::gateway::GatewayHttpState,
    litellm_pricing_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(path) = litellm_pricing_path else {
        return Ok(state);
    };
    let raw = std::fs::read_to_string(path)?;
    let pricing = ditto_llm::gateway::PricingTable::from_litellm_json_str(&raw)?;
    Ok(state.with_pricing_table(pricing))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-costing")))]
pub(crate) fn attach_pricing_table(
    state: ditto_llm::gateway::GatewayHttpState,
    litellm_pricing_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if litellm_pricing_path.is_some() {
        return Err("pricing requires `--features gateway-costing`".into());
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-metrics-prometheus"))]
pub(crate) fn attach_prometheus_metrics(
    state: ditto_llm::gateway::GatewayHttpState,
    enabled: bool,
    max_key_series: Option<usize>,
    max_model_series: Option<usize>,
    max_backend_series: Option<usize>,
    max_path_series: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(state);
    }

    let mut config = ditto_llm::gateway::metrics_prometheus::PrometheusMetricsConfig::default();
    if let Some(value) = max_key_series {
        config.max_key_series = value.max(1);
    }
    if let Some(value) = max_model_series {
        config.max_model_series = value.max(1);
    }
    if let Some(value) = max_backend_series {
        config.max_backend_series = value.max(1);
    }
    if let Some(value) = max_path_series {
        config.max_path_series = value.max(1);
    }
    Ok(state.with_prometheus_metrics(config))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-metrics-prometheus")))]
pub(crate) fn attach_prometheus_metrics(
    state: ditto_llm::gateway::GatewayHttpState,
    enabled: bool,
    max_key_series: Option<usize>,
    max_model_series: Option<usize>,
    max_backend_series: Option<usize>,
    max_path_series: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if enabled
        || max_key_series.is_some()
        || max_model_series.is_some()
        || max_backend_series.is_some()
        || max_path_series.is_some()
    {
        return Err("prometheus metrics requires `--features gateway-metrics-prometheus`".into());
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-otel"))]
pub(crate) fn attach_otel(
    enabled: bool,
    endpoint: Option<&str>,
    json_logs: bool,
) -> Result<Option<ditto_llm::gateway::otel::OtelGuard>, Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(None);
    }

    Ok(Some(ditto_llm::gateway::otel::init_tracing(
        "ditto-gateway",
        endpoint,
        json_logs,
    )?))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-otel")))]
pub(crate) fn attach_otel(
    enabled: bool,
    _endpoint: Option<&str>,
    _json_logs: bool,
) -> Result<Option<()>, Box<dyn std::error::Error>> {
    if enabled {
        return Err("otel requires `--features gateway-otel`".into());
    }
    Ok(None)
}
