#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or(
        "usage: ditto-gateway <config.json> [--dotenv PATH] [--listen HOST:PORT] [--admin-token TOKEN] [--admin-token-env ENV] [--state PATH] [--sqlite PATH] [--redis URL] [--redis-env ENV] [--redis-prefix PREFIX] [--backend name=url] [--upstream name=base_url] [--json-logs] [--proxy-cache] [--proxy-cache-ttl SECS] [--proxy-cache-max-entries N] [--proxy-cache-max-body-bytes N] [--proxy-cache-max-total-body-bytes N] [--proxy-max-in-flight N] [--proxy-retry] [--proxy-retry-status-codes CODES] [--proxy-retry-max-attempts N] [--proxy-circuit-breaker] [--proxy-cb-failure-threshold N] [--proxy-cb-cooldown-secs SECS] [--proxy-health-checks] [--proxy-health-check-path PATH] [--proxy-health-check-interval-secs SECS] [--proxy-health-check-timeout-secs SECS] [--pricing-litellm PATH] [--prometheus-metrics] [--prometheus-max-key-series N] [--prometheus-max-model-series N] [--prometheus-max-backend-series N] [--prometheus-max-path-series N] [--devtools PATH] [--otel] [--otel-endpoint URL] [--otel-json]",
    )?;

    let mut listen = "127.0.0.1:8080".to_string();
    let mut admin_token: Option<String> = None;
    let mut admin_token_env: Option<String> = None;
    let mut dotenv_path: Option<std::path::PathBuf> = None;
    let mut state_path: Option<std::path::PathBuf> = None;
    let mut _sqlite_path: Option<std::path::PathBuf> = None;
    let mut redis_url: Option<String> = None;
    let mut redis_url_env: Option<String> = None;
    let mut redis_prefix: Option<String> = None;
    let mut backend_specs: Vec<String> = Vec::new();
    let mut upstream_specs: Vec<String> = Vec::new();
    let mut json_logs = false;
    let mut proxy_cache_enabled = false;
    let mut proxy_cache_ttl_seconds: Option<u64> = None;
    let mut proxy_cache_max_entries: Option<usize> = None;
    let mut proxy_cache_max_body_bytes: Option<usize> = None;
    let mut proxy_cache_max_total_body_bytes: Option<usize> = None;
    let mut proxy_max_in_flight: Option<usize> = None;
    let mut pricing_litellm_path: Option<String> = None;
    let mut prometheus_metrics_enabled = false;
    let mut prometheus_max_key_series: Option<usize> = None;
    let mut prometheus_max_model_series: Option<usize> = None;
    let mut prometheus_max_backend_series: Option<usize> = None;
    let mut prometheus_max_path_series: Option<usize> = None;
    let mut proxy_retry_enabled = false;
    let mut proxy_retry_status_codes: Option<Vec<u16>> = None;
    let mut proxy_retry_max_attempts: Option<usize> = None;
    let mut proxy_circuit_breaker_enabled = false;
    let mut proxy_cb_failure_threshold: Option<u32> = None;
    let mut proxy_cb_cooldown_secs: Option<u64> = None;
    let mut proxy_health_checks_enabled = false;
    let mut proxy_health_check_path: Option<String> = None;
    let mut proxy_health_check_interval_secs: Option<u64> = None;
    let mut proxy_health_check_timeout_secs: Option<u64> = None;
    let mut devtools_path: Option<String> = None;
    let mut otel_enabled = false;
    let mut otel_endpoint: Option<String> = None;
    let mut otel_json = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" | "--addr" => {
                listen = args.next().ok_or("missing value for --listen/--addr")?;
            }
            "--dotenv" => {
                dotenv_path = Some(args.next().ok_or("missing value for --dotenv")?.into());
            }
            "--admin-token" => {
                admin_token = Some(args.next().ok_or("missing value for --admin-token")?);
            }
            "--admin-token-env" => {
                admin_token_env = Some(args.next().ok_or("missing value for --admin-token-env")?);
            }
            "--state" => {
                state_path = Some(args.next().ok_or("missing value for --state")?.into());
            }
            "--sqlite" => {
                _sqlite_path = Some(args.next().ok_or("missing value for --sqlite")?.into());
            }
            "--redis" => {
                redis_url = Some(args.next().ok_or("missing value for --redis")?);
            }
            "--redis-env" => {
                redis_url_env = Some(args.next().ok_or("missing value for --redis-env")?);
            }
            "--redis-prefix" => {
                redis_prefix = Some(args.next().ok_or("missing value for --redis-prefix")?);
            }
            "--backend" => {
                backend_specs.push(args.next().ok_or("missing value for --backend")?);
            }
            "--upstream" => {
                upstream_specs.push(args.next().ok_or("missing value for --upstream")?);
            }
            "--json-logs" => {
                json_logs = true;
            }
            "--proxy-cache" => {
                proxy_cache_enabled = true;
            }
            "--proxy-cache-ttl" => {
                proxy_cache_enabled = true;
                let raw = args.next().ok_or("missing value for --proxy-cache-ttl")?;
                proxy_cache_ttl_seconds = Some(
                    raw.parse::<u64>()
                        .map_err(|_| "invalid --proxy-cache-ttl")?,
                );
            }
            "--proxy-cache-max-entries" => {
                proxy_cache_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-cache-max-entries")?;
                proxy_cache_max_entries = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-cache-max-entries")?,
                );
            }
            "--proxy-cache-max-body-bytes" => {
                proxy_cache_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-cache-max-body-bytes")?;
                proxy_cache_max_body_bytes = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-cache-max-body-bytes")?,
                );
            }
            "--proxy-cache-max-total-body-bytes" => {
                proxy_cache_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-cache-max-total-body-bytes")?;
                proxy_cache_max_total_body_bytes = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-cache-max-total-body-bytes")?,
                );
            }
            "--proxy-max-in-flight" => {
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-max-in-flight")?;
                proxy_max_in_flight = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-max-in-flight")?,
                );
            }
            "--pricing-litellm" => {
                pricing_litellm_path =
                    Some(args.next().ok_or("missing value for --pricing-litellm")?);
            }
            "--prometheus-metrics" => {
                prometheus_metrics_enabled = true;
            }
            "--prometheus-max-key-series" => {
                prometheus_metrics_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --prometheus-max-key-series")?;
                prometheus_max_key_series = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --prometheus-max-key-series")?,
                );
            }
            "--prometheus-max-model-series" => {
                prometheus_metrics_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --prometheus-max-model-series")?;
                prometheus_max_model_series = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --prometheus-max-model-series")?,
                );
            }
            "--prometheus-max-backend-series" => {
                prometheus_metrics_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --prometheus-max-backend-series")?;
                prometheus_max_backend_series = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --prometheus-max-backend-series")?,
                );
            }
            "--prometheus-max-path-series" => {
                prometheus_metrics_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --prometheus-max-path-series")?;
                prometheus_max_path_series = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --prometheus-max-path-series")?,
                );
            }
            "--proxy-retry" => {
                proxy_retry_enabled = true;
            }
            "--proxy-retry-status-codes" => {
                proxy_retry_enabled = true;
                proxy_retry_status_codes = Some(parse_status_codes(
                    &args
                        .next()
                        .ok_or("missing value for --proxy-retry-status-codes")?,
                )?);
            }
            "--proxy-retry-max-attempts" => {
                proxy_retry_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-retry-max-attempts")?;
                proxy_retry_max_attempts = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-retry-max-attempts")?,
                );
            }
            "--proxy-circuit-breaker" => {
                proxy_circuit_breaker_enabled = true;
            }
            "--proxy-cb-failure-threshold" => {
                proxy_circuit_breaker_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-cb-failure-threshold")?;
                proxy_cb_failure_threshold = Some(
                    raw.parse::<u32>()
                        .map_err(|_| "invalid --proxy-cb-failure-threshold")?,
                );
            }
            "--proxy-cb-cooldown-secs" => {
                proxy_circuit_breaker_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-cb-cooldown-secs")?;
                proxy_cb_cooldown_secs = Some(
                    raw.parse::<u64>()
                        .map_err(|_| "invalid --proxy-cb-cooldown-secs")?,
                );
            }
            "--proxy-health-checks" => {
                proxy_health_checks_enabled = true;
            }
            "--proxy-health-check-path" => {
                proxy_health_checks_enabled = true;
                proxy_health_check_path = Some(
                    args.next()
                        .ok_or("missing value for --proxy-health-check-path")?,
                );
            }
            "--proxy-health-check-interval-secs" => {
                proxy_health_checks_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-health-check-interval-secs")?;
                proxy_health_check_interval_secs = Some(
                    raw.parse::<u64>()
                        .map_err(|_| "invalid --proxy-health-check-interval-secs")?,
                );
            }
            "--proxy-health-check-timeout-secs" => {
                proxy_health_checks_enabled = true;
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-health-check-timeout-secs")?;
                proxy_health_check_timeout_secs = Some(
                    raw.parse::<u64>()
                        .map_err(|_| "invalid --proxy-health-check-timeout-secs")?,
                );
            }
            "--devtools" => {
                devtools_path = Some(args.next().ok_or("missing value for --devtools")?);
            }
            "--otel" => {
                otel_enabled = true;
            }
            "--otel-endpoint" => {
                otel_enabled = true;
                otel_endpoint = Some(args.next().ok_or("missing value for --otel-endpoint")?);
            }
            "--otel-json" => {
                otel_enabled = true;
                otel_json = true;
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let env = if let Some(path) = dotenv_path.as_deref() {
        let raw = std::fs::read_to_string(path)?;
        ditto_llm::Env::parse_dotenv(&raw)
    } else {
        ditto_llm::Env {
            dotenv: std::collections::BTreeMap::new(),
        }
    };

    if admin_token.is_some() && admin_token_env.is_some() {
        return Err("--admin-token cannot be combined with --admin-token-env".into());
    }
    if redis_url.is_some() && redis_url_env.is_some() {
        return Err("--redis cannot be combined with --redis-env".into());
    }

    if let Some(key) = admin_token_env.as_deref() {
        let token = env
            .get(key)
            .ok_or_else(|| format!("missing env var for --admin-token-env: {key}"))?;
        if token.trim().is_empty() {
            return Err(format!("admin token env var is empty: {key}").into());
        }
        admin_token = Some(token);
    }

    if let Some(key) = redis_url_env.as_deref() {
        let url = env
            .get(key)
            .ok_or_else(|| format!("missing env var for --redis-env: {key}"))?;
        if url.trim().is_empty() {
            return Err(format!("redis url env var is empty: {key}").into());
        }
        redis_url = Some(url);
    }

    let storage_count =
        state_path.is_some() as u8 + _sqlite_path.is_some() as u8 + redis_url.is_some() as u8;
    if storage_count > 1 {
        return Err("use only one of --state, --sqlite, or --redis".into());
    }
    if redis_prefix.is_some() && redis_url.is_none() {
        return Err("--redis-prefix requires --redis or --redis-env".into());
    }

    let raw = std::fs::read_to_string(&path)?;
    let mut config: ditto_llm::gateway::GatewayConfig = serde_json::from_str(&raw)?;

    if let Some(_sqlite_path_ref) = _sqlite_path.as_ref() {
        #[cfg(feature = "gateway-store-sqlite")]
        {
            let existed = _sqlite_path_ref.exists();
            let store = ditto_llm::gateway::SqliteStore::new(_sqlite_path_ref);
            store.init().await?;
            if existed {
                config.virtual_keys = store.load_virtual_keys().await?;
            } else {
                store.replace_virtual_keys(&config.virtual_keys).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-sqlite"))]
        {
            return Err("sqlite store requires `--features gateway-store-sqlite`".into());
        }
    }

    if let Some(_redis_url_ref) = redis_url.as_ref() {
        #[cfg(feature = "gateway-store-redis")]
        {
            let mut store = ditto_llm::gateway::RedisStore::new(_redis_url_ref)?;
            if let Some(prefix) = redis_prefix.as_ref() {
                store = store.with_prefix(prefix.clone());
            }
            store.ping().await?;
            let loaded = store.load_virtual_keys().await?;
            if !loaded.is_empty() {
                config.virtual_keys = loaded;
            } else {
                store.replace_virtual_keys(&config.virtual_keys).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-redis"))]
        {
            return Err("redis store requires `--features gateway-store-redis`".into());
        }
    }

    if let Some(state_path) = state_path.as_ref() {
        if state_path.exists() {
            let state = ditto_llm::gateway::GatewayStateFile::load(state_path)?;
            config.virtual_keys = state.virtual_keys;
        } else {
            ditto_llm::gateway::GatewayStateFile {
                virtual_keys: config.virtual_keys.clone(),
            }
            .save(state_path)?;
        }
    }

    config.resolve_env(&env)?;

    for key in &config.virtual_keys {
        if let Err(err) = key.guardrails.validate() {
            return Err(format!("invalid guardrails config for key {}: {err}", key.id).into());
        }
    }

    for rule in &config.router.rules {
        if let Some(guardrails) = rule.guardrails.as_ref() {
            if let Err(err) = guardrails.validate() {
                return Err(format!(
                    "invalid guardrails config for route {}: {err}",
                    rule.model_prefix
                )
                .into());
            }
        }
    }

    let mut proxy_backends = std::collections::HashMap::new();
    #[cfg(feature = "gateway-translation")]
    let mut translation_backends = std::collections::HashMap::new();

    #[cfg(feature = "gateway-translation")]
    for backend in &config.backends {
        if backend
            .provider
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty())
        {
            if !backend.base_url.trim().is_empty() {
                return Err(format!(
                    "backend {} cannot set both base_url and provider",
                    backend.name
                )
                .into());
            }

            #[cfg(feature = "gateway-translation")]
            {
                let provider = backend.provider.as_deref().unwrap_or_default();
                let provider_config = backend.provider_config.clone().unwrap_or_default();
                let model = ditto_llm::gateway::translation::build_language_model(
                    provider,
                    &provider_config,
                    &env,
                )
                .await?;
                let backend_model = ditto_llm::gateway::TranslationBackend::new(provider, model)
                    .with_env(env.clone())
                    .with_provider_config(provider_config)
                    .with_model_map(backend.model_map.clone());
                if translation_backends
                    .insert(backend.name.clone(), backend_model)
                    .is_some()
                {
                    return Err(format!("duplicate backend name: {}", backend.name).into());
                }
            }
            #[cfg(not(feature = "gateway-translation"))]
            {
                return Err("provider backend requires `--features gateway-translation`".into());
            }

            continue;
        }

        if backend.base_url.trim().is_empty() {
            return Err(format!("backend {} missing base_url", backend.name).into());
        }

        let mut client = ditto_llm::gateway::ProxyBackend::new(&backend.base_url)?;
        client = client.with_headers(backend.headers.clone())?;
        client = client.with_query_params(backend.query_params.clone());
        client = client.with_request_timeout_seconds(backend.timeout_seconds);
        if proxy_backends
            .insert(backend.name.clone(), client)
            .is_some()
        {
            return Err(format!("duplicate backend name: {}", backend.name).into());
        }
    }

    for spec in upstream_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or("upstream spec must be name=base_url")?;
        let client = ditto_llm::gateway::ProxyBackend::new(url)?;
        proxy_backends.insert(name.to_string(), client);
    }

    let mut gateway = ditto_llm::gateway::Gateway::new(config);

    for spec in backend_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or("backend spec must be name=url")?;
        let backend = ditto_llm::gateway::HttpBackend::new(url)?;
        gateway.register_backend(name.to_string(), backend);
    }

    let mut state =
        ditto_llm::gateway::GatewayHttpState::new(gateway).with_proxy_backends(proxy_backends);
    #[cfg(feature = "gateway-translation")]
    {
        state = state.with_translation_backends(translation_backends);
    }
    if let Some(token) = admin_token {
        state = state.with_admin_token(token);
    }
    if json_logs {
        state = state.with_json_logs();
    }
    state = attach_proxy_cache(
        state,
        proxy_cache_enabled,
        proxy_cache_ttl_seconds,
        proxy_cache_max_entries,
        proxy_cache_max_body_bytes,
        proxy_cache_max_total_body_bytes,
    )?;
    state = attach_proxy_backpressure(state, proxy_max_in_flight)?;
    state = attach_pricing_table(state, pricing_litellm_path)?;
    state = attach_prometheus_metrics(
        state,
        prometheus_metrics_enabled,
        prometheus_max_key_series,
        prometheus_max_model_series,
        prometheus_max_backend_series,
        prometheus_max_path_series,
    )?;
    state = attach_proxy_routing(
        state,
        ProxyRoutingCliOptions {
            retry_enabled: proxy_retry_enabled,
            retry_status_codes: proxy_retry_status_codes,
            retry_max_attempts: proxy_retry_max_attempts,
            circuit_breaker_enabled: proxy_circuit_breaker_enabled,
            cb_failure_threshold: proxy_cb_failure_threshold,
            cb_cooldown_secs: proxy_cb_cooldown_secs,
            health_checks_enabled: proxy_health_checks_enabled,
            health_check_path: proxy_health_check_path,
            health_check_interval_secs: proxy_health_check_interval_secs,
            health_check_timeout_secs: proxy_health_check_timeout_secs,
        },
    )?;
    if let Some(path) = state_path {
        state = state.with_state_file(path);
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(path) = _sqlite_path {
        state = state.with_sqlite_store(ditto_llm::gateway::SqliteStore::new(path));
    }
    #[cfg(feature = "gateway-store-redis")]
    if let Some(redis_url) = redis_url {
        let mut store = ditto_llm::gateway::RedisStore::new(redis_url)?;
        if let Some(prefix) = redis_prefix {
            store = store.with_prefix(prefix);
        }
        state = state.with_redis_store(store);
    }
    state = attach_devtools(state, devtools_path)?;

    let _otel_guard = attach_otel(otel_enabled, otel_endpoint.as_deref(), otel_json)?;

    let app = ditto_llm::gateway::http::router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    println!("ditto-gateway listening on {listen}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(all(feature = "gateway", feature = "sdk"))]
fn attach_devtools(
    state: ditto_llm::gateway::GatewayHttpState,
    devtools_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(path) = devtools_path else {
        return Ok(state);
    };
    Ok(state.with_devtools_logger(ditto_llm::sdk::devtools::DevtoolsLogger::new(path)))
}

#[cfg(all(feature = "gateway", not(feature = "sdk")))]
fn attach_devtools(
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
fn attach_proxy_backpressure(
    state: ditto_llm::gateway::GatewayHttpState,
    max_in_flight: Option<usize>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    let Some(max) = max_in_flight else {
        return Ok(state);
    };
    if max == 0 {
        return Err("--proxy-max-in-flight must be > 0".into());
    }
    Ok(state.with_proxy_max_in_flight(max))
}

#[cfg(all(feature = "gateway", feature = "gateway-proxy-cache"))]
fn attach_proxy_cache(
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
fn attach_proxy_cache(
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
struct ProxyRoutingCliOptions {
    retry_enabled: bool,
    retry_status_codes: Option<Vec<u16>>,
    retry_max_attempts: Option<usize>,
    circuit_breaker_enabled: bool,
    cb_failure_threshold: Option<u32>,
    cb_cooldown_secs: Option<u64>,
    health_checks_enabled: bool,
    health_check_path: Option<String>,
    health_check_interval_secs: Option<u64>,
    health_check_timeout_secs: Option<u64>,
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
fn attach_proxy_routing(
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
fn attach_proxy_routing(
    state: ditto_llm::gateway::GatewayHttpState,
    opts: ProxyRoutingCliOptions,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if opts.any_set() {
        return Err("proxy routing requires `--features gateway-routing-advanced`".into());
    }
    Ok(state)
}

#[cfg(feature = "gateway")]
fn parse_status_codes(raw: &str) -> Result<Vec<u16>, Box<dyn std::error::Error>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty status code list".into());
    }

    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(part.parse::<u16>().map_err(|_| "invalid status code")?);
    }
    if out.is_empty() {
        return Err("empty status code list".into());
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(all(feature = "gateway", feature = "gateway-costing"))]
fn attach_pricing_table(
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
fn attach_pricing_table(
    state: ditto_llm::gateway::GatewayHttpState,
    litellm_pricing_path: Option<String>,
) -> Result<ditto_llm::gateway::GatewayHttpState, Box<dyn std::error::Error>> {
    if litellm_pricing_path.is_some() {
        return Err("pricing requires `--features gateway-costing`".into());
    }
    Ok(state)
}

#[cfg(all(feature = "gateway", feature = "gateway-metrics-prometheus"))]
fn attach_prometheus_metrics(
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
fn attach_prometheus_metrics(
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
fn attach_otel(
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
fn attach_otel(
    enabled: bool,
    _endpoint: Option<&str>,
    _json_logs: bool,
) -> Result<Option<()>, Box<dyn std::error::Error>> {
    if enabled {
        return Err("otel requires `--features gateway-otel`".into());
    }
    Ok(None)
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("gateway feature disabled; rebuild with --features gateway");
}
