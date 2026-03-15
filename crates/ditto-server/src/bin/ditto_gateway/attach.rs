#[cfg(feature = "gateway")]
use ditto_core::MESSAGE_CATALOG;
#[cfg(feature = "gateway")]
use ditto_core::i18n::{Locale, MessageArg, MessageCatalogExt as _};

#[cfg(all(feature = "gateway", feature = "gateway-routing-advanced"))]
fn parse_proxy_failure_action(
    raw: &str,
    locale: Locale,
) -> Result<ditto_server::gateway::proxy_routing::ProxyFailureAction, Box<dyn std::error::Error>> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(ditto_server::gateway::proxy_routing::ProxyFailureAction::None),
        "fallback" => Ok(ditto_server::gateway::proxy_routing::ProxyFailureAction::Fallback),
        "retry" => Ok(ditto_server::gateway::proxy_routing::ProxyFailureAction::Retry),
        _ => Err(invalid_value(locale, "proxy failure action")),
    }
}

#[cfg(all(feature = "gateway", feature = "sdk"))]
pub(crate) fn attach_devtools(
    state: ditto_server::gateway::GatewayHttpState,
    devtools_path: Option<String>,
    _locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(path) = devtools_path else {
        return Ok(state);
    };
    Ok(state.with_devtools_logger(ditto_core::sdk::devtools::DevtoolsLogger::new(path)))
}

#[cfg(all(feature = "gateway", not(feature = "sdk")))]
pub(crate) fn attach_devtools(
    state: ditto_server::gateway::GatewayHttpState,
    devtools_path: Option<String>,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if devtools_path.is_some() {
        return Err(feature_disabled(
            locale,
            "devtools",
            "--features gateway-devtools (or `sdk` alongside `gateway`)",
        ));
    }
    Ok(state)
}

#[cfg(feature = "gateway")]
const DEFAULT_PROXY_MAX_IN_FLIGHT: usize = 256;

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_backpressure(
    state: ditto_server::gateway::GatewayHttpState,
    max_in_flight: Option<usize>,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let max = max_in_flight.unwrap_or(DEFAULT_PROXY_MAX_IN_FLIGHT);
    if max == 0 {
        return Err(must_be_positive(locale, "--proxy-max-in-flight"));
    }
    Ok(state.with_proxy_max_in_flight(max))
}

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_max_body_bytes(
    state: ditto_server::gateway::GatewayHttpState,
    max_body_bytes: Option<usize>,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(max) = max_body_bytes else {
        return Ok(state);
    };
    if max == 0 {
        return Err(must_be_positive(locale, "--proxy-max-body-bytes"));
    }
    Ok(state.with_proxy_max_body_bytes(max))
}

#[cfg(feature = "gateway")]
pub(crate) fn attach_proxy_usage_max_body_bytes(
    state: ditto_server::gateway::GatewayHttpState,
    max_body_bytes: Option<usize>,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(max) = max_body_bytes else {
        return Ok(state);
    };
    Ok(state.with_proxy_usage_max_body_bytes(max))
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "gateway-proxy-cache"), allow(dead_code))]
#[derive(Default)]
pub(crate) struct ProxyCacheCliOptions {
    pub(crate) enabled: bool,
    pub(crate) ttl_seconds: Option<u64>,
    pub(crate) max_entries: Option<usize>,
    pub(crate) max_body_bytes: Option<usize>,
    pub(crate) max_total_body_bytes: Option<usize>,
    pub(crate) streaming_enabled: bool,
    pub(crate) max_stream_body_bytes: Option<usize>,
}

#[cfg(all(feature = "gateway", feature = "gateway-proxy-cache"))]
pub(crate) fn attach_proxy_cache(
    state: ditto_server::gateway::GatewayHttpState,
    opts: ProxyCacheCliOptions,
    _locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !opts.enabled {
        return Ok(state);
    }

    let mut config = ditto_server::gateway::ProxyCacheConfig::default();
    config.ttl_seconds = opts.ttl_seconds.unwrap_or(config.ttl_seconds).max(1);
    config.max_entries = opts.max_entries.unwrap_or(config.max_entries).max(1);
    if let Some(max_body_bytes) = opts.max_body_bytes {
        config.max_body_bytes = max_body_bytes.max(1);
    }
    if let Some(max_total_body_bytes) = opts.max_total_body_bytes {
        config.max_total_body_bytes = max_total_body_bytes.max(1);
    }
    config.streaming_enabled = opts.streaming_enabled;
    if let Some(max_stream_body_bytes) = opts.max_stream_body_bytes {
        config.max_stream_body_bytes = max_stream_body_bytes.max(1);
    }
    Ok(state.with_proxy_cache(config))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-proxy-cache")))]
pub(crate) fn attach_proxy_cache(
    state: ditto_server::gateway::GatewayHttpState,
    opts: ProxyCacheCliOptions,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if opts.enabled {
        return Err(feature_disabled(
            locale,
            "proxy cache",
            "--features gateway-proxy-cache",
        ));
    }
    Ok(state)
}

#[cfg(feature = "gateway")]
#[derive(Default)]
pub(crate) struct ProxyRoutingCliOptions {
    pub(crate) retry_enabled: bool,
    pub(crate) retry_status_codes: Option<Vec<u16>>,
    pub(crate) fallback_status_codes: Option<Vec<u16>>,
    pub(crate) network_error_action: Option<String>,
    pub(crate) timeout_error_action: Option<String>,
    pub(crate) retry_max_attempts: Option<usize>,
    pub(crate) circuit_breaker_enabled: bool,
    pub(crate) cb_failure_threshold: Option<u32>,
    pub(crate) cb_cooldown_secs: Option<u64>,
    pub(crate) cb_failure_status_codes: Option<Vec<u16>>,
    pub(crate) cb_no_network_errors: bool,
    pub(crate) cb_no_timeout_errors: bool,
    pub(crate) cb_no_server_errors: bool,
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
            || self.fallback_status_codes.is_some()
            || self.network_error_action.is_some()
            || self.timeout_error_action.is_some()
            || self.retry_max_attempts.is_some()
            || self.circuit_breaker_enabled
            || self.cb_failure_threshold.is_some()
            || self.cb_cooldown_secs.is_some()
            || self.cb_failure_status_codes.is_some()
            || self.cb_no_network_errors
            || self.cb_no_timeout_errors
            || self.cb_no_server_errors
            || self.health_checks_enabled
            || self.health_check_path.is_some()
            || self.health_check_interval_secs.is_some()
            || self.health_check_timeout_secs.is_some()
    }
}

#[cfg(all(feature = "gateway", feature = "gateway-routing-advanced"))]
pub(crate) fn attach_proxy_routing(
    state: ditto_server::gateway::GatewayHttpState,
    opts: ProxyRoutingCliOptions,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !opts.any_set() {
        return Ok(state);
    }

    let mut config = ditto_server::gateway::ProxyRoutingConfig::default();
    if opts.retry_enabled {
        config.retry.enabled = true;
    }
    if let Some(codes) = opts.retry_status_codes {
        config.retry.retry_status_codes = codes;
    }
    if let Some(codes) = opts.fallback_status_codes {
        config.retry.fallback_status_codes = codes;
    }
    if let Some(action) = opts.network_error_action.as_deref() {
        config.retry.network_error_action = parse_proxy_failure_action(action, locale)?;
    }
    if let Some(action) = opts.timeout_error_action.as_deref() {
        config.retry.timeout_error_action = parse_proxy_failure_action(action, locale)?;
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
    if let Some(codes) = opts.cb_failure_status_codes {
        config.circuit_breaker.failure_status_codes = codes;
    }
    if opts.cb_no_network_errors {
        config.circuit_breaker.count_network_errors = false;
    }
    if opts.cb_no_timeout_errors {
        config.circuit_breaker.count_timeout_errors = false;
    }
    if opts.cb_no_server_errors {
        config.circuit_breaker.count_server_errors = false;
    }

    if opts.health_checks_enabled {
        config.health_check.enabled = true;
    }
    if let Some(path) = opts.health_check_path {
        if path.trim().is_empty() {
            return Err(non_empty_required(locale, "health check path"));
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
    state: ditto_server::gateway::GatewayHttpState,
    opts: ProxyRoutingCliOptions,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if opts.any_set() {
        return Err(feature_disabled(
            locale,
            "proxy routing",
            "--features gateway-routing-advanced",
        ));
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-costing"))]
pub(crate) fn attach_pricing_table(
    state: ditto_server::gateway::GatewayHttpState,
    litellm_pricing_path: Option<String>,
    _locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(path) = litellm_pricing_path else {
        return Ok(state);
    };
    let raw = std::fs::read_to_string(path)?;
    let pricing = ditto_server::gateway::PricingTable::from_litellm_json_str(&raw)?;
    Ok(state.with_pricing_table(pricing))
}

#[cfg(all(feature = "gateway", not(feature = "gateway-costing")))]
pub(crate) fn attach_pricing_table(
    state: ditto_server::gateway::GatewayHttpState,
    litellm_pricing_path: Option<String>,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if litellm_pricing_path.is_some() {
        return Err(feature_disabled(
            locale,
            "pricing",
            "--features gateway-costing",
        ));
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-metrics-prometheus"))]
pub(crate) fn attach_prometheus_metrics(
    state: ditto_server::gateway::GatewayHttpState,
    enabled: bool,
    max_key_series: Option<usize>,
    max_model_series: Option<usize>,
    max_backend_series: Option<usize>,
    max_path_series: Option<usize>,
    _locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(state);
    }

    let mut config = ditto_server::gateway::metrics_prometheus::PrometheusMetricsConfig::default();
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
    state: ditto_server::gateway::GatewayHttpState,
    enabled: bool,
    max_key_series: Option<usize>,
    max_model_series: Option<usize>,
    max_backend_series: Option<usize>,
    max_path_series: Option<usize>,
    locale: Locale,
) -> Result<ditto_server::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if enabled
        || max_key_series.is_some()
        || max_model_series.is_some()
        || max_backend_series.is_some()
        || max_path_series.is_some()
    {
        return Err(feature_disabled(
            locale,
            "prometheus metrics",
            "--features gateway-metrics-prometheus",
        ));
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-otel"))]
pub(crate) fn attach_otel(
    enabled: bool,
    endpoint: Option<&str>,
    json_logs: bool,
    _locale: Locale,
) -> Result<Option<ditto_server::gateway::otel::OtelGuard>, Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(None);
    }

    Ok(Some(ditto_server::gateway::otel::init_tracing(
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
    locale: Locale,
) -> Result<Option<()>, Box<dyn std::error::Error>> {
    if enabled {
        return Err(feature_disabled(locale, "otel", "--features gateway-otel"));
    }
    Ok(None)
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "gateway-routing-advanced"), allow(dead_code))]
fn invalid_value(locale: Locale, label: &str) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.invalid_value",
            &[MessageArg::new("label", label)],
        )
        .into()
}

#[cfg(feature = "gateway")]
fn must_be_positive(locale: Locale, flag: &str) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.must_be_positive",
            &[MessageArg::new("flag", flag)],
        )
        .into()
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "gateway-routing-advanced"), allow(dead_code))]
fn non_empty_required(locale: Locale, label: &str) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.non_empty_required",
            &[MessageArg::new("label", label)],
        )
        .into()
}

#[cfg(feature = "gateway")]
#[allow(dead_code)]
fn feature_disabled(
    locale: Locale,
    feature: &str,
    rebuild_hint: &str,
) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.feature_disabled",
            &[
                MessageArg::new("feature", feature),
                MessageArg::new("rebuild_hint", rebuild_hint),
            ],
        )
        .into()
}
