#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    const DEFAULT_AUDIT_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

    let cli = parse_gateway_cli_args(std::env::args().skip(1))?;
    let GatewayCliArgs {
        path,
        listen,
        mut admin_token,
        admin_token_env,
        mut admin_read_token,
        admin_read_token_env,
        mut admin_tenant_tokens,
        admin_tenant_token_env,
        mut admin_tenant_read_tokens,
        admin_tenant_read_token_env,
        dotenv_path,
        state_path,
        sqlite_path: _sqlite_path,
        mut redis_url,
        redis_url_env,
        redis_prefix,
        audit_retention_secs: _audit_retention_secs,
        backend_specs,
        upstream_specs,
        json_logs,
        proxy_cache_enabled,
        proxy_cache_ttl_seconds,
        proxy_cache_max_entries,
        proxy_cache_max_body_bytes,
        proxy_cache_max_total_body_bytes,
        proxy_max_body_bytes,
        proxy_usage_max_body_bytes,
        proxy_max_in_flight,
        pricing_litellm_path,
        prometheus_metrics_enabled,
        prometheus_max_key_series,
        prometheus_max_model_series,
        prometheus_max_backend_series,
        prometheus_max_path_series,
        proxy_retry_enabled,
        proxy_retry_status_codes,
        proxy_retry_max_attempts,
        proxy_circuit_breaker_enabled,
        proxy_cb_failure_threshold,
        proxy_cb_cooldown_secs,
        proxy_health_checks_enabled,
        proxy_health_check_path,
        proxy_health_check_interval_secs,
        proxy_health_check_timeout_secs,
        devtools_path,
        otel_enabled,
        otel_endpoint,
        otel_json,
    } = cli;
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let audit_retention_secs = _audit_retention_secs;

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
    if admin_read_token.is_some() && admin_read_token_env.is_some() {
        return Err("--admin-read-token cannot be combined with --admin-read-token-env".into());
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

    if let Some(key) = admin_read_token_env.as_deref() {
        let token = env
            .get(key)
            .ok_or_else(|| format!("missing env var for --admin-read-token-env: {key}"))?;
        if token.trim().is_empty() {
            return Err(format!("admin read token env var is empty: {key}").into());
        }
        admin_read_token = Some(token);
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, env_key) in &admin_tenant_token_env {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err("admin tenant token env spec has empty tenant id".into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(
                format!("duplicate --admin-tenant-token-env tenant id: {tenant_id}").into(),
            );
        }
        let token = env
            .get(env_key)
            .ok_or_else(|| format!("missing env var for --admin-tenant-token-env: {env_key}"))?;
        if token.trim().is_empty() {
            return Err(format!("admin tenant token env var is empty: {env_key}").into());
        }
        admin_tenant_tokens.push((tenant_id.to_string(), token));
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, env_key) in &admin_tenant_read_token_env {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err("admin tenant read token env spec has empty tenant id".into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(
                format!("duplicate --admin-tenant-read-token-env tenant id: {tenant_id}").into(),
            );
        }
        let token = env.get(env_key).ok_or_else(|| {
            format!("missing env var for --admin-tenant-read-token-env: {env_key}")
        })?;
        if token.trim().is_empty() {
            return Err(format!("admin tenant read token env var is empty: {env_key}").into());
        }
        admin_tenant_read_tokens.push((tenant_id.to_string(), token));
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, _token) in &admin_tenant_tokens {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err("admin tenant token has empty tenant id".into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(format!("duplicate --admin-tenant-token tenant id: {tenant_id}").into());
        }
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, _token) in &admin_tenant_read_tokens {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err("admin tenant read token has empty tenant id".into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(
                format!("duplicate --admin-tenant-read-token tenant id: {tenant_id}").into(),
            );
        }
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

    if let Some(token) = admin_token.take() {
        admin_token = Some(resolve_cli_secret(token, &env, "admin token").await?);
    }
    if let Some(token) = admin_read_token.take() {
        admin_read_token = Some(resolve_cli_secret(token, &env, "admin read token").await?);
    }
    for (_tenant_id, token) in &mut admin_tenant_tokens {
        let raw = std::mem::take(token);
        *token = resolve_cli_secret(raw, &env, "admin tenant token").await?;
    }
    for (_tenant_id, token) in &mut admin_tenant_read_tokens {
        let raw = std::mem::take(token);
        *token = resolve_cli_secret(raw, &env, "admin tenant read token").await?;
    }
    if let Some(url) = redis_url.take() {
        redis_url = Some(resolve_cli_secret(url, &env, "redis url").await?);
    }

    let storage_count =
        state_path.is_some() as u8 + _sqlite_path.is_some() as u8 + redis_url.is_some() as u8;
    if storage_count > 1 {
        return Err("use only one of --state, --sqlite, or --redis".into());
    }
    if redis_prefix.is_some() && redis_url.is_none() {
        return Err("--redis-prefix requires --redis or --redis-env".into());
    }

    let config_path = std::path::Path::new(&path);
    let raw = std::fs::read_to_string(config_path)?;
    let config_ext = config_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let mut config: ditto_llm::gateway::GatewayConfig = match config_ext.as_deref() {
        Some("yaml") | Some("yml") => {
            #[cfg(feature = "gateway-config-yaml")]
            {
                match serde_yaml::from_str::<ditto_llm::gateway::GatewayConfig>(&raw) {
                    Ok(config) => config,
                    Err(gateway_yaml_err) => {
                        let litellm_config = serde_yaml::from_str::<
                            ditto_llm::gateway::litellm_config::LitellmProxyConfig,
                        >(&raw)
                        .map_err(|litellm_err| {
                            format!(
                                "failed to parse config as ditto gateway yaml ({gateway_yaml_err}) or litellm proxy yaml ({litellm_err})"
                            )
                        })?;
                        litellm_config.try_into_gateway_config().map_err(|err| {
                            format!("failed to import litellm proxy config: {err}")
                        })?
                    }
                }
            }
            #[cfg(not(feature = "gateway-config-yaml"))]
            {
                return Err("yaml config requires `--features gateway-config-yaml`".into());
            }
        }
        _ => match serde_json::from_str(&raw) {
            Ok(config) => config,
            Err(json_err) => {
                #[cfg(feature = "gateway-config-yaml")]
                {
                    match serde_yaml::from_str::<ditto_llm::gateway::GatewayConfig>(&raw) {
                        Ok(config) => config,
                        Err(gateway_yaml_err) => {
                            let litellm_config = serde_yaml::from_str::<
                                ditto_llm::gateway::litellm_config::LitellmProxyConfig,
                            >(&raw)
                            .map_err(|litellm_err| {
                                format!(
                                    "failed to parse config as json ({json_err}), ditto gateway yaml ({gateway_yaml_err}), or litellm proxy yaml ({litellm_err})"
                                )
                            })?;
                            litellm_config.try_into_gateway_config().map_err(|err| {
                                format!("failed to import litellm proxy config: {err}")
                            })?
                        }
                    }
                }
                #[cfg(not(feature = "gateway-config-yaml"))]
                {
                    return Err(format!(
                        "failed to parse config as json ({json_err}); yaml requires `--features gateway-config-yaml`"
                    )
                    .into());
                }
            }
        },
    };

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
    config.resolve_secrets(&env).await?;

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
                continue;
            }
            #[cfg(not(feature = "gateway-translation"))]
            {
                return Err("provider backend requires `--features gateway-translation`".into());
            }
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

    let mut a2a_agents = std::collections::HashMap::new();
    for agent in &config.a2a_agents {
        let agent_id = agent.agent_id.trim();
        if agent_id.is_empty() {
            return Err("a2a agent_id is empty".into());
        }

        let url = agent
            .agent_card_params
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if url.is_empty() {
            return Err(format!("a2a agent {agent_id} missing agent_card_params.url").into());
        }

        let mut client = ditto_llm::gateway::ProxyBackend::new(&url)?;
        client = client.with_headers(agent.headers.clone())?;
        client = client.with_query_params(agent.query_params.clone());
        client = client.with_request_timeout_seconds(agent.timeout_seconds);

        let agent_state = ditto_llm::gateway::http::A2aAgentState::new(
            agent_id.to_string(),
            agent.agent_card_params.clone(),
            client,
        );
        if a2a_agents
            .insert(agent_id.to_string(), agent_state)
            .is_some()
        {
            return Err(format!("duplicate a2a agent_id: {agent_id}").into());
        }
    }

    let mut mcp_servers = std::collections::HashMap::new();
    for server in &config.mcp_servers {
        let server_id = server.server_id.trim();
        if server_id.is_empty() {
            return Err("mcp server_id is empty".into());
        }

        let url = server.url.trim();
        if url.is_empty() {
            return Err(format!("mcp server {server_id} missing url").into());
        }

        let mut client =
            ditto_llm::gateway::http::McpServerState::new(server_id.to_string(), url.to_string())?;
        client = client.with_headers(server.headers.clone())?;
        client = client.with_query_params(server.query_params.clone());
        client = client.with_request_timeout_seconds(server.timeout_seconds);

        if mcp_servers.insert(server_id.to_string(), client).is_some() {
            return Err(format!("duplicate mcp server_id: {server_id}").into());
        }
    }

    let mut gateway = ditto_llm::gateway::Gateway::new(config);

    for spec in backend_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or("backend spec must be name=url")?;
        let backend = ditto_llm::gateway::HttpBackend::new(url)?;
        gateway.register_backend(name.to_string(), backend);
    }

    let mut state = ditto_llm::gateway::GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_a2a_agents(a2a_agents)
        .with_mcp_servers(mcp_servers);
    #[cfg(feature = "gateway-translation")]
    {
        state = state.with_translation_backends(translation_backends);
    }
    if let Some(token) = admin_token {
        state = state.with_admin_token(token);
    }
    if let Some(token) = admin_read_token {
        state = state.with_admin_read_token(token);
    }
    for (tenant_id, token) in admin_tenant_tokens {
        state = state.with_admin_tenant_token(tenant_id, token);
    }
    for (tenant_id, token) in admin_tenant_read_tokens {
        state = state.with_admin_tenant_read_token(tenant_id, token);
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
    state = attach_proxy_max_body_bytes(state, proxy_max_body_bytes)?;
    state = attach_proxy_usage_max_body_bytes(state, proxy_usage_max_body_bytes)?;
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
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let effective_audit_retention_secs = match audit_retention_secs {
        Some(0) => None,
        Some(secs) => Some(secs),
        None if _sqlite_path.is_some() || redis_url.is_some() => Some(DEFAULT_AUDIT_RETENTION_SECS),
        None => None,
    };
    if let Some(path) = state_path {
        state = state.with_state_file(path);
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(path) = _sqlite_path {
        state = state.with_sqlite_store(
            ditto_llm::gateway::SqliteStore::new(path)
                .with_audit_retention_secs(effective_audit_retention_secs),
        );
    }
    #[cfg(feature = "gateway-store-redis")]
    if let Some(redis_url) = redis_url {
        let mut store = ditto_llm::gateway::RedisStore::new(redis_url)?;
        if let Some(prefix) = redis_prefix {
            store = store.with_prefix(prefix);
        }
        store = store.with_audit_retention_secs(effective_audit_retention_secs);
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

include!("ditto_gateway/cli.rs");

include!("ditto_gateway/attach.rs");

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("gateway feature disabled; rebuild with --features gateway");
}
