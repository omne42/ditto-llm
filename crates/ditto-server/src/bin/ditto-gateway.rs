#[cfg(feature = "gateway")]
mod ditto_gateway;

#[cfg(feature = "gateway")]
use ditto_gateway::attach::{
    ProxyCacheCliOptions, ProxyRoutingCliOptions, attach_devtools, attach_otel,
    attach_pricing_table, attach_prometheus_metrics, attach_proxy_backpressure, attach_proxy_cache,
    attach_proxy_max_body_bytes, attach_proxy_routing, attach_proxy_usage_max_body_bytes,
};

#[cfg(feature = "gateway")]
use config_kit::{ConfigDocument, ConfigFormat, ConfigLoadOptions, load_config_document};
use ditto_core::resources::MESSAGE_CATALOG;
#[cfg(feature = "gateway")]
use ditto_gateway::cli::{
    GatewayCliArgs, gateway_cli_usage, parse_gateway_cli_args_with_locale, resolve_cli_secret,
};
#[cfg(feature = "gateway")]
use ditto_gateway::config_cli::maybe_run_config_cli;
use i18n_kit::{Locale, TemplateArg};

#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    let data_root = match ditto_server::data_root::bootstrap_cli_runtime_from_args(&raw_args) {
        Ok(data_root) => data_root,
        Err(err) => {
            eprintln!("{err:?}");
            std::process::exit(2);
        }
    };
    let (locale, raw_args) = match MESSAGE_CATALOG.resolve_cli_locale(raw_args, "DITTO_LOCALE") {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };
    let raw_args =
        ditto_server::data_root::inject_default_gateway_config_path(raw_args, &data_root);

    if let Err(err) = run_gateway(locale, raw_args).await {
        if let Some(localized) = err
            .as_ref()
            .downcast_ref::<ditto_gateway::clap_i18n::LocalizedCliError>()
        {
            eprintln!("{localized}");
        } else {
            eprintln!("{}", render_error(err.as_ref(), locale));
        }
        std::process::exit(1);
    }
}

#[cfg(feature = "gateway")]
async fn run_gateway(
    locale: Locale,
    raw_args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    const DEFAULT_AUDIT_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

    if raw_args.is_empty()
        || (!matches!(
            raw_args.first().map(String::as_str),
            Some("provider" | "model")
        ) && raw_args.iter().any(|arg| arg == "--help" || arg == "-h"))
    {
        println!("{}", gateway_cli_usage(locale));
        return Ok(());
    }

    if maybe_run_config_cli(raw_args.clone(), locale).await? {
        return Ok(());
    }
    let cli = parse_gateway_cli_args_with_locale(raw_args.into_iter(), locale)?;
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
        mut postgres_url,
        postgres_url_env,
        mut mysql_url,
        mysql_url_env,
        mut redis_url,
        redis_url_env,
        redis_prefix,
        audit_retention_secs: _audit_retention_secs,
        db_doctor,
        backend_specs,
        upstream_specs,
        json_logs,
        proxy_cache_enabled,
        proxy_cache_ttl_seconds,
        proxy_cache_max_entries,
        proxy_cache_max_body_bytes,
        proxy_cache_max_total_body_bytes,
        proxy_cache_streaming_enabled,
        proxy_cache_max_stream_body_bytes,
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
        proxy_fallback_status_codes,
        proxy_network_error_action,
        proxy_timeout_error_action,
        proxy_retry_max_attempts,
        proxy_circuit_breaker_enabled,
        proxy_cb_failure_threshold,
        proxy_cb_cooldown_secs,
        proxy_cb_failure_status_codes,
        proxy_cb_no_network_errors,
        proxy_cb_no_timeout_errors,
        proxy_cb_no_server_errors,
        proxy_health_checks_enabled,
        proxy_health_check_path,
        proxy_health_check_interval_secs,
        proxy_health_check_timeout_secs,
        devtools_path,
        otel_enabled,
        otel_endpoint,
        otel_json,
    } = cli;
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let audit_retention_secs = _audit_retention_secs;

    let env = if let Some(path) = dotenv_path.as_deref() {
        let raw = std::fs::read_to_string(path)?;
        ditto_core::config::Env::parse_dotenv(&raw)
    } else {
        ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::new(),
        }
    };

    if admin_token.is_some() && admin_token_env.is_some() {
        return Err(cli_cannot_combine(locale, "--admin-token", "--admin-token-env").into());
    }
    if admin_read_token.is_some() && admin_read_token_env.is_some() {
        return Err(
            cli_cannot_combine(locale, "--admin-read-token", "--admin-read-token-env").into(),
        );
    }
    if redis_url.is_some() && redis_url_env.is_some() {
        return Err(cli_cannot_combine(locale, "--redis", "--redis-env").into());
    }
    if postgres_url.is_some() && postgres_url_env.is_some() {
        return Err(
            cli_cannot_combine(locale, "--pg/--postgres", "--pg-env/--postgres-env").into(),
        );
    }
    if mysql_url.is_some() && mysql_url_env.is_some() {
        return Err(cli_cannot_combine(locale, "--mysql", "--mysql-env").into());
    }

    if let Some(key) = admin_token_env.as_deref() {
        let token = env
            .get(key)
            .ok_or_else(|| cli_missing_env(locale, "--admin-token-env", key))?;
        if token.trim().is_empty() {
            return Err(cli_env_empty(locale, "admin token env var", key).into());
        }
        admin_token = Some(token);
    }

    if let Some(key) = admin_read_token_env.as_deref() {
        let token = env
            .get(key)
            .ok_or_else(|| cli_missing_env(locale, "--admin-read-token-env", key))?;
        if token.trim().is_empty() {
            return Err(cli_env_empty(locale, "admin read token env var", key).into());
        }
        admin_read_token = Some(token);
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, env_key) in &admin_tenant_token_env {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err(cli_empty_value(locale, "admin tenant token env spec tenant id").into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(cli_duplicate_value(
                locale,
                "--admin-tenant-token-env tenant id",
                tenant_id,
            )
            .into());
        }
        let token = env
            .get(env_key)
            .ok_or_else(|| cli_missing_env(locale, "--admin-tenant-token-env", env_key))?;
        if token.trim().is_empty() {
            return Err(cli_env_empty(locale, "admin tenant token env var", env_key).into());
        }
        admin_tenant_tokens.push((tenant_id.to_string(), token));
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, env_key) in &admin_tenant_read_token_env {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err(
                cli_empty_value(locale, "admin tenant read token env spec tenant id").into(),
            );
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(cli_duplicate_value(
                locale,
                "--admin-tenant-read-token-env tenant id",
                tenant_id,
            )
            .into());
        }
        let token = env
            .get(env_key)
            .ok_or_else(|| cli_missing_env(locale, "--admin-tenant-read-token-env", env_key))?;
        if token.trim().is_empty() {
            return Err(cli_env_empty(locale, "admin tenant read token env var", env_key).into());
        }
        admin_tenant_read_tokens.push((tenant_id.to_string(), token));
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, _token) in &admin_tenant_tokens {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err(cli_empty_value(locale, "admin tenant token tenant id").into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(
                cli_duplicate_value(locale, "--admin-tenant-token tenant id", tenant_id).into(),
            );
        }
    }

    let mut seen_tenants = std::collections::HashSet::<String>::new();
    for (tenant_id, _token) in &admin_tenant_read_tokens {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() {
            return Err(cli_empty_value(locale, "admin tenant read token tenant id").into());
        }
        if !seen_tenants.insert(tenant_id.to_string()) {
            return Err(cli_duplicate_value(
                locale,
                "--admin-tenant-read-token tenant id",
                tenant_id,
            )
            .into());
        }
    }

    if let Some(key) = redis_url_env.as_deref() {
        let url = env
            .get(key)
            .ok_or_else(|| cli_missing_env(locale, "--redis-env", key))?;
        if url.trim().is_empty() {
            return Err(cli_env_empty(locale, "redis url env var", key).into());
        }
        redis_url = Some(url);
    }
    if let Some(key) = postgres_url_env.as_deref() {
        let url = env
            .get(key)
            .ok_or_else(|| cli_missing_env(locale, "--pg-env", key))?;
        if url.trim().is_empty() {
            return Err(cli_env_empty(locale, "postgres url env var", key).into());
        }
        postgres_url = Some(url);
    }
    if let Some(key) = mysql_url_env.as_deref() {
        let url = env
            .get(key)
            .ok_or_else(|| cli_missing_env(locale, "--mysql-env", key))?;
        if url.trim().is_empty() {
            return Err(cli_env_empty(locale, "mysql url env var", key).into());
        }
        mysql_url = Some(url);
    }

    if let Some(token) = admin_token.take() {
        admin_token = Some(resolve_cli_secret(token, &env, "admin token", locale).await?);
    }
    if let Some(token) = admin_read_token.take() {
        admin_read_token = Some(resolve_cli_secret(token, &env, "admin read token", locale).await?);
    }
    for (_tenant_id, token) in &mut admin_tenant_tokens {
        let raw = std::mem::take(token);
        *token = resolve_cli_secret(raw, &env, "admin tenant token", locale).await?;
    }
    for (_tenant_id, token) in &mut admin_tenant_read_tokens {
        let raw = std::mem::take(token);
        *token = resolve_cli_secret(raw, &env, "admin tenant read token", locale).await?;
    }
    if let Some(url) = redis_url.take() {
        redis_url = Some(resolve_cli_secret(url, &env, "redis url", locale).await?);
    }
    if let Some(url) = postgres_url.take() {
        postgres_url = Some(resolve_cli_secret(url, &env, "postgres url", locale).await?);
    }
    if let Some(url) = mysql_url.take() {
        mysql_url = Some(resolve_cli_secret(url, &env, "mysql url", locale).await?);
    }
    if redis_prefix.is_some() && redis_url.is_none() {
        return Err(cli_requires(locale, "--redis-prefix", "--redis or --redis-env").into());
    }
    validate_single_control_plane_persistence_target(
        locale,
        state_path.is_some(),
        _sqlite_path.is_some(),
        postgres_url.is_some(),
        mysql_url.is_some(),
        redis_url.is_some(),
    )?;

    let mut config = load_gateway_config(locale, &path)?;

    if let Some(_sqlite_path_ref) = _sqlite_path.as_ref() {
        #[cfg(feature = "gateway-store-sqlite")]
        {
            let existed = _sqlite_path_ref.exists();
            let store = ditto_server::gateway::SqliteStore::new(_sqlite_path_ref);
            store.init().await?;
            store.verify_schema().await?;
            if existed {
                config.virtual_keys = store.load_virtual_keys().await?;
                if let Some(router) = store.load_router_config().await? {
                    config.router = router;
                } else {
                    store.replace_router_config(&config.router).await?;
                }
            } else {
                store.replace_virtual_keys(&config.virtual_keys).await?;
                store.replace_router_config(&config.router).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-sqlite"))]
        {
            return Err(cli_feature_disabled(
                locale,
                "sqlite store",
                "--features gateway-store-sqlite",
            )
            .into());
        }
    }

    if let Some(_postgres_url_ref) = postgres_url.as_ref() {
        #[cfg(feature = "gateway-store-postgres")]
        {
            let store = ditto_server::gateway::PostgresStore::connect(_postgres_url_ref).await?;
            store.ping().await?;
            store.init().await?;
            store.verify_schema().await?;

            let loaded_keys = store.load_virtual_keys().await?;
            let loaded_router = store.load_router_config().await?;
            if loaded_router.is_some() || !loaded_keys.is_empty() {
                config.virtual_keys = loaded_keys;
                if let Some(router) = loaded_router {
                    config.router = router;
                } else {
                    store.replace_router_config(&config.router).await?;
                }
            } else {
                store.replace_router_config(&config.router).await?;
                store.replace_virtual_keys(&config.virtual_keys).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-postgres"))]
        {
            return Err(cli_feature_disabled(
                locale,
                "postgres store",
                "--features gateway-store-postgres",
            )
            .into());
        }
    }

    if let Some(_mysql_url_ref) = mysql_url.as_ref() {
        #[cfg(feature = "gateway-store-mysql")]
        {
            let store = ditto_server::gateway::MySqlStore::connect(_mysql_url_ref).await?;
            store.ping().await?;
            store.init().await?;
            store.verify_schema().await?;

            let loaded_keys = store.load_virtual_keys().await?;
            let loaded_router = store.load_router_config().await?;
            if loaded_router.is_some() || !loaded_keys.is_empty() {
                config.virtual_keys = loaded_keys;
                if let Some(router) = loaded_router {
                    config.router = router;
                } else {
                    store.replace_router_config(&config.router).await?;
                }
            } else {
                store.replace_router_config(&config.router).await?;
                store.replace_virtual_keys(&config.virtual_keys).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-mysql"))]
        {
            return Err(cli_feature_disabled(
                locale,
                "mysql store",
                "--features gateway-store-mysql",
            )
            .into());
        }
    }

    if let Some(_redis_url_ref) = redis_url.as_ref() {
        #[cfg(feature = "gateway-store-redis")]
        {
            let mut store = ditto_server::gateway::RedisStore::new(_redis_url_ref)?;
            if let Some(prefix) = redis_prefix.as_ref() {
                store = store.with_prefix(prefix.clone());
            }
            store.ping().await?;
            let loaded_keys = store.load_virtual_keys().await?;
            let loaded_router = store.load_router_config().await?;
            if loaded_router.is_some() || !loaded_keys.is_empty() {
                config.virtual_keys = loaded_keys;
                if let Some(router) = loaded_router {
                    config.router = router;
                } else {
                    store.replace_router_config(&config.router).await?;
                }
            } else {
                store.replace_virtual_keys(&config.virtual_keys).await?;
                store.replace_router_config(&config.router).await?;
            }
        }
        #[cfg(not(feature = "gateway-store-redis"))]
        {
            return Err(cli_feature_disabled(
                locale,
                "redis store",
                "--features gateway-store-redis",
            )
            .into());
        }
    }

    if db_doctor {
        if _sqlite_path.is_none()
            && postgres_url.is_none()
            && mysql_url.is_none()
            && redis_url.is_none()
        {
            return Err(cli_requires(
                locale,
                "--db-doctor",
                "at least one store flag (--sqlite/--pg/--mysql/--redis)",
            )
            .into());
        }
        println!("{}", cli_schema_checks_passed(locale));
        return Ok(());
    }

    if let Some(state_path) = state_path.as_ref() {
        if state_path.exists() {
            let state = ditto_server::gateway::GatewayStateFile::load(state_path)?;
            config.virtual_keys = state.virtual_keys;
            if let Some(router) = state.router {
                config.router = router;
            }
        } else {
            ditto_server::gateway::GatewayStateFile {
                virtual_keys: config.virtual_keys.clone(),
                router: Some(config.router.clone()),
            }
            .save(state_path)?;
        }
    }

    config.resolve_env(&env)?;
    config.resolve_secrets(&env).await?;

    config.validate()?;

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
                return Err(cli_cannot_set_both(
                    locale,
                    "backend",
                    &backend.name,
                    "base_url",
                    "provider",
                )
                .into());
            }

            #[cfg(feature = "gateway-translation")]
            {
                let provider = backend.provider.as_deref().unwrap_or_default();
                let provider_config = backend.provider_config.clone().unwrap_or_default();
                let model =
                    ditto_core::runtime::build_language_model(provider, &provider_config, &env)
                        .await?;
                let backend_model = ditto_server::gateway::TranslationBackend::new(provider, model)
                    .with_env(env.clone())
                    .with_provider_config(provider_config)
                    .with_model_map(backend.model_map.clone());
                if translation_backends
                    .insert(backend.name.clone(), backend_model)
                    .is_some()
                {
                    return Err(cli_duplicate_value(locale, "backend name", &backend.name).into());
                }
                continue;
            }
            #[cfg(not(feature = "gateway-translation"))]
            {
                return Err(cli_feature_disabled(
                    locale,
                    "provider backend",
                    "--features gateway-translation",
                )
                .into());
            }
        }

        if backend.base_url.trim().is_empty() {
            return Err(cli_missing_field(locale, "backend", &backend.name, "base_url").into());
        }

        let mut client = ditto_server::gateway::ProxyBackend::new(&backend.base_url)?;
        client = client.with_headers(backend.headers.clone())?;
        client = client.with_query_params(backend.query_params.clone());
        client = client.with_request_timeout_seconds(backend.timeout_seconds);
        if proxy_backends
            .insert(backend.name.clone(), client)
            .is_some()
        {
            return Err(cli_duplicate_value(locale, "backend name", &backend.name).into());
        }
    }

    for spec in upstream_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or_else(|| cli_invalid_spec(locale, "upstream spec", "name=base_url"))?;
        let client = ditto_server::gateway::ProxyBackend::new(url)?;
        proxy_backends.insert(name.to_string(), client);
    }

    let mut a2a_agents = std::collections::HashMap::new();
    for agent in &config.a2a_agents {
        let agent_id = agent.agent_id.trim();
        if agent_id.is_empty() {
            return Err(cli_empty_value(locale, "a2a agent_id").into());
        }

        let url = agent
            .agent_card_params
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if url.is_empty() {
            return Err(
                cli_missing_field(locale, "a2a agent", agent_id, "agent_card_params.url").into(),
            );
        }

        let mut client = ditto_server::gateway::ProxyBackend::new(&url)?;
        client = client.with_headers(agent.headers.clone())?;
        client = client.with_query_params(agent.query_params.clone());
        client = client.with_request_timeout_seconds(agent.timeout_seconds);

        let agent_state = ditto_server::gateway::http::A2aAgentState::new(
            agent_id.to_string(),
            agent.agent_card_params.clone(),
            client,
        );
        if a2a_agents
            .insert(agent_id.to_string(), agent_state)
            .is_some()
        {
            return Err(cli_duplicate_value(locale, "a2a agent_id", agent_id).into());
        }
    }

    let mut mcp_servers = std::collections::HashMap::new();
    for server in &config.mcp_servers {
        let server_id = server.server_id.trim();
        if server_id.is_empty() {
            return Err(cli_empty_value(locale, "mcp server_id").into());
        }

        let url = server.url.trim();
        if url.is_empty() {
            return Err(cli_missing_field(locale, "mcp server", server_id, "url").into());
        }

        let mut client = ditto_server::gateway::http::McpServerState::new(
            server_id.to_string(),
            url.to_string(),
        )?;
        client = client.with_headers(server.headers.clone())?;
        client = client.with_query_params(server.query_params.clone());
        client = client.with_request_timeout_seconds(server.timeout_seconds);

        if mcp_servers.insert(server_id.to_string(), client).is_some() {
            return Err(cli_duplicate_value(locale, "mcp server_id", server_id).into());
        }
    }

    let mut gateway = ditto_server::gateway::Gateway::new(config);

    for spec in backend_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or_else(|| cli_invalid_spec(locale, "backend spec", "name=url"))?;
        let backend = ditto_server::gateway::HttpBackend::new(url)?;
        gateway.register_backend(name.to_string(), backend);
    }

    let mut state = ditto_server::gateway::GatewayHttpState::new(gateway)
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
        ProxyCacheCliOptions {
            enabled: proxy_cache_enabled,
            ttl_seconds: proxy_cache_ttl_seconds,
            max_entries: proxy_cache_max_entries,
            max_body_bytes: proxy_cache_max_body_bytes,
            max_total_body_bytes: proxy_cache_max_total_body_bytes,
            streaming_enabled: proxy_cache_streaming_enabled,
            max_stream_body_bytes: proxy_cache_max_stream_body_bytes,
        },
        locale,
    )?;
    state = attach_proxy_max_body_bytes(state, proxy_max_body_bytes, locale)?;
    state = attach_proxy_usage_max_body_bytes(state, proxy_usage_max_body_bytes)?;
    state = attach_proxy_backpressure(state, proxy_max_in_flight, locale)?;
    state = attach_pricing_table(state, pricing_litellm_path, locale)?;
    state = attach_prometheus_metrics(
        state,
        prometheus_metrics_enabled,
        prometheus_max_key_series,
        prometheus_max_model_series,
        prometheus_max_backend_series,
        prometheus_max_path_series,
        locale,
    )?;
    state = attach_proxy_routing(
        state,
        ProxyRoutingCliOptions {
            retry_enabled: proxy_retry_enabled,
            retry_status_codes: proxy_retry_status_codes,
            fallback_status_codes: proxy_fallback_status_codes,
            network_error_action: proxy_network_error_action,
            timeout_error_action: proxy_timeout_error_action,
            retry_max_attempts: proxy_retry_max_attempts,
            circuit_breaker_enabled: proxy_circuit_breaker_enabled,
            cb_failure_threshold: proxy_cb_failure_threshold,
            cb_cooldown_secs: proxy_cb_cooldown_secs,
            cb_failure_status_codes: proxy_cb_failure_status_codes,
            cb_no_network_errors: proxy_cb_no_network_errors,
            cb_no_timeout_errors: proxy_cb_no_timeout_errors,
            cb_no_server_errors: proxy_cb_no_server_errors,
            health_checks_enabled: proxy_health_checks_enabled,
            health_check_path: proxy_health_check_path,
            health_check_interval_secs: proxy_health_check_interval_secs,
            health_check_timeout_secs: proxy_health_check_timeout_secs,
        },
        locale,
    )?;
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let effective_audit_retention_secs = match audit_retention_secs {
        Some(0) => None,
        Some(secs) => Some(secs),
        None if _sqlite_path.is_some()
            || postgres_url.is_some()
            || mysql_url.is_some()
            || redis_url.is_some() =>
        {
            Some(DEFAULT_AUDIT_RETENTION_SECS)
        }
        None => None,
    };
    if let Some(path) = state_path {
        state = state.with_state_file(path);
    }
    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(path) = _sqlite_path {
        let store = ditto_server::gateway::SqliteStore::new(path)
            .with_audit_retention_secs(effective_audit_retention_secs);
        store.verify_schema().await?;
        state = state.with_sqlite_store(store);
    }
    #[cfg(feature = "gateway-store-postgres")]
    if let Some(postgres_url) = postgres_url {
        let store = ditto_server::gateway::PostgresStore::connect(postgres_url)
            .await?
            .with_audit_retention_secs(effective_audit_retention_secs);
        store.verify_schema().await?;
        state = state.with_postgres_store(store);
    }
    #[cfg(feature = "gateway-store-mysql")]
    if let Some(mysql_url) = mysql_url {
        let store = ditto_server::gateway::MySqlStore::connect(mysql_url)
            .await?
            .with_audit_retention_secs(effective_audit_retention_secs);
        store.verify_schema().await?;
        state = state.with_mysql_store(store);
    }
    #[cfg(feature = "gateway-store-redis")]
    if let Some(redis_url) = redis_url {
        let mut store = ditto_server::gateway::RedisStore::new(redis_url)?;
        if let Some(prefix) = redis_prefix {
            store = store.with_prefix(prefix);
        }
        store = store.with_audit_retention_secs(effective_audit_retention_secs);
        state = state.with_redis_store(store);
    }
    state = attach_devtools(state, devtools_path, locale)?;

    let _otel_guard = attach_otel(otel_enabled, otel_endpoint.as_deref(), otel_json, locale)?;

    let app = ditto_server::gateway::http::router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    println!("{}", cli_listening_on(locale, &listen));
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(feature = "gateway")]
fn load_gateway_config(
    locale: Locale,
    path: &str,
) -> Result<ditto_server::gateway::GatewayConfig, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(path);
    let document = load_gateway_config_document(locale, path)?;
    parse_gateway_config_document(locale, &document)
}

#[cfg(feature = "gateway")]
fn load_gateway_config_document(
    _locale: Locale,
    path: &std::path::Path,
) -> Result<ConfigDocument, Box<dyn std::error::Error>> {
    let options = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("json") => {
            ConfigLoadOptions::new().with_format(ConfigFormat::Json)
        }
        Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") => {
            #[cfg(feature = "gateway-config-yaml")]
            {
                ConfigLoadOptions::new().with_format(ConfigFormat::Yaml)
            }
            #[cfg(not(feature = "gateway-config-yaml"))]
            {
                return Err(cli_feature_disabled(
                    _locale,
                    "yaml config",
                    "--features gateway-config-yaml",
                )
                .into());
            }
        }
        Some(ext) => {
            let expected = if cfg!(feature = "gateway-config-yaml") {
                ".json, .yaml, or .yml"
            } else {
                ".json"
            };
            return Err(format!(
                "unsupported gateway config extension .{ext} for {}: expected {expected} or no extension",
                path.display()
            )
            .into());
        }
        None => ConfigLoadOptions::new().with_default_format(ConfigFormat::Json),
    };

    load_config_document(path, options).map_err(|err| err.to_string().into())
}

#[cfg(feature = "gateway")]
fn parse_gateway_config_document(
    locale: Locale,
    document: &ConfigDocument,
) -> Result<ditto_server::gateway::GatewayConfig, Box<dyn std::error::Error>> {
    match document.format() {
        ConfigFormat::Json => document
            .parse::<ditto_server::gateway::GatewayConfig>()
            .map_err(|err| err.to_string().into()),
        ConfigFormat::Yaml => {
            #[cfg(feature = "gateway-config-yaml")]
            {
                match document.parse::<ditto_server::gateway::GatewayConfig>() {
                    Ok(config) => Ok(config),
                    Err(gateway_yaml_err) => {
                        let litellm_config = ConfigFormat::Yaml
                            .parse_with_path::<ditto_server::gateway::litellm_config::LitellmProxyConfig>(
                                document.contents(),
                                Some(document.path()),
                            )
                            .map_err(|litellm_err| {
                                cli_parse_config_failed(
                                    locale,
                                    &format!("ditto gateway yaml ({gateway_yaml_err})"),
                                    &format!("litellm proxy yaml ({litellm_err})"),
                                )
                            })?;
                        litellm_config.try_into_gateway_config().map_err(|err| {
                            cli_import_litellm_failed(locale, &err.to_string()).into()
                        })
                    }
                }
            }
            #[cfg(not(feature = "gateway-config-yaml"))]
            {
                Err(
                    cli_feature_disabled(locale, "yaml config", "--features gateway-config-yaml")
                        .into(),
                )
            }
        }
        other => Err(format!(
            "unsupported gateway config format {other} for {}",
            document.path().display()
        )
        .into()),
    }
}

#[cfg(feature = "gateway")]
fn cli_cannot_combine(locale: Locale, left: &str, right: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.cannot_combine",
        &[
            TemplateArg::new("left", left),
            TemplateArg::new("right", right),
        ],
    )
}

#[cfg(feature = "gateway")]
fn validate_single_control_plane_persistence_target(
    locale: Locale,
    has_state: bool,
    has_sqlite: bool,
    has_postgres: bool,
    has_mysql: bool,
    has_redis: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let configured = [has_state, has_sqlite, has_postgres, has_mysql, has_redis]
        .into_iter()
        .filter(|configured| *configured)
        .count();

    if configured > 1 {
        return Err(format!(
            "{}: {}",
            MESSAGE_CATALOG.render(
                locale,
                "cli.invalid_value",
                &[TemplateArg::new(
                    "label",
                    "control-plane persistence target"
                )],
            ),
            "choose exactly one of --state, --sqlite, --pg/--postgres, --mysql, or --redis"
        )
        .into());
    }

    Ok(())
}

#[cfg(feature = "gateway")]
fn cli_requires(locale: Locale, flag: &str, requirement: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.requires",
        &[
            TemplateArg::new("flag", flag),
            TemplateArg::new("requirement", requirement),
        ],
    )
}

#[cfg(feature = "gateway")]
fn cli_missing_env(locale: Locale, flag: &str, key: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.missing_env",
        &[TemplateArg::new("flag", flag), TemplateArg::new("key", key)],
    )
}

#[cfg(feature = "gateway")]
fn cli_env_empty(locale: Locale, label: &str, key: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.env_empty",
        &[
            TemplateArg::new("label", label),
            TemplateArg::new("key", key),
        ],
    )
}

#[cfg(feature = "gateway")]
fn cli_empty_value(locale: Locale, label: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.empty_value",
        &[TemplateArg::new("label", label)],
    )
}

#[cfg(feature = "gateway")]
fn cli_duplicate_value(locale: Locale, label: &str, value: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.duplicate_value",
        &[
            TemplateArg::new("label", label),
            TemplateArg::new("value", value),
        ],
    )
}

#[cfg(feature = "gateway")]
#[allow(dead_code)]
fn cli_json_parse_then_yaml_disabled(locale: Locale, json_error: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.json_parse_then_yaml_disabled",
        &[TemplateArg::new("json_error", json_error)],
    )
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "gateway-config-yaml"), allow(dead_code))]
fn cli_parse_config_failed(locale: Locale, primary: &str, secondary: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.parse_config_failed",
        &[
            TemplateArg::new("primary", primary),
            TemplateArg::new("secondary", secondary),
        ],
    )
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(feature = "gateway-config-yaml"), allow(dead_code))]
fn cli_import_litellm_failed(locale: Locale, error: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.import_litellm_failed",
        &[TemplateArg::new("error", error)],
    )
}

#[cfg(feature = "gateway")]
fn cli_missing_field(locale: Locale, scope: &str, name: &str, field: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.missing_field",
        &[
            TemplateArg::new("scope", scope),
            TemplateArg::new("name", name),
            TemplateArg::new("field", field),
        ],
    )
}

#[cfg(feature = "gateway")]
fn cli_cannot_set_both(locale: Locale, scope: &str, name: &str, left: &str, right: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.cannot_set_both",
        &[
            TemplateArg::new("scope", scope),
            TemplateArg::new("name", name),
            TemplateArg::new("left", left),
            TemplateArg::new("right", right),
        ],
    )
}

#[cfg(feature = "gateway")]
fn cli_invalid_spec(locale: Locale, label: &str, expected: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.invalid_spec",
        &[
            TemplateArg::new("label", label),
            TemplateArg::new("expected", expected),
        ],
    )
}

#[allow(dead_code)]
fn cli_feature_disabled(locale: Locale, feature: &str, rebuild_hint: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.feature_disabled",
        &[
            TemplateArg::new("feature", feature),
            TemplateArg::new("rebuild_hint", rebuild_hint),
        ],
    )
}

#[cfg(feature = "gateway")]
fn cli_schema_checks_passed(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "cli.schema_checks_passed", &[])
}

#[cfg(feature = "gateway")]
fn cli_listening_on(locale: Locale, listen: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.listening_on",
        &[TemplateArg::new("listen", listen)],
    )
}

#[cfg(feature = "gateway")]
fn render_error(error: &(dyn std::error::Error + 'static), locale: Locale) -> String {
    if let Some(error) = error.downcast_ref::<ditto_core::error::DittoError>() {
        return error.render(locale);
    }
    if let Some(error) = error.downcast_ref::<ditto_core::error::ProviderResolutionError>() {
        return error.render(locale);
    }
    MESSAGE_CATALOG.render(
        locale,
        "error.generic",
        &[TemplateArg::new("error", error.to_string())],
    )
}

#[cfg(all(feature = "gateway", test))]
mod tests {
    use super::*;

    fn test_locale() -> Locale {
        MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US)
    }

    fn write_temp_file(name: &str, contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write config");
        (dir, path)
    }

    #[test]
    fn load_gateway_config_accepts_strict_json() {
        let raw = serde_json::to_string(&ditto_server::gateway::GatewayConfig::default())
            .expect("serialize json");
        let (_dir, path) = write_temp_file("gateway.json", &raw);
        let config = load_gateway_config(test_locale(), path.to_str().expect("utf8 path"))
            .expect("load json config");
        assert!(config.backends.is_empty());
        assert!(config.virtual_keys.is_empty());
        assert!(config.router.default_backends.is_empty());
    }

    #[test]
    fn load_gateway_config_rejects_yaml_payload_in_json_file() {
        let raw = r#"
virtual_keys: []
router:
  default_backends: []
  rules: []
"#;
        let (_dir, path) = write_temp_file("gateway.json", raw);
        let err = load_gateway_config(test_locale(), path.to_str().expect("utf8 path"))
            .expect_err("json path must not accept yaml payload");
        assert!(err.to_string().contains("failed to parse json config"));
    }

    #[test]
    fn gateway_config_validate_rejects_unknown_router_backend_before_runtime() {
        let config = ditto_server::gateway::GatewayConfig {
            backends: Vec::new(),
            virtual_keys: vec![ditto_server::gateway::VirtualKeyConfig::new(
                "key-1", "vk-1",
            )],
            router: ditto_server::gateway::RouterConfig {
                default_backends: vec![ditto_server::gateway::RouteBackend {
                    backend: "missing-backend".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        let err = config
            .validate()
            .expect_err("startup validation must fail before serving traffic");
        assert!(matches!(
            err,
            ditto_server::gateway::GatewayError::InvalidRequest { reason }
                if reason.contains("router references unknown backends: missing-backend")
        ));
    }

    #[test]
    fn gateway_config_validate_accepts_provider_backends_as_router_targets() {
        let config = ditto_server::gateway::GatewayConfig {
            backends: vec![ditto_server::gateway::BackendConfig {
                name: "primary".to_string(),
                base_url: String::new(),
                max_in_flight: None,
                timeout_seconds: None,
                headers: std::collections::BTreeMap::new(),
                query_params: std::collections::BTreeMap::new(),
                provider: Some("openai-compatible".to_string()),
                provider_config: Some(ditto_core::config::ProviderConfig {
                    base_url: Some("https://proxy.example/v1".to_string()),
                    default_model: Some("gpt-4o-mini".to_string()),
                    ..Default::default()
                }),
                model_map: std::collections::BTreeMap::new(),
            }],
            virtual_keys: vec![ditto_server::gateway::VirtualKeyConfig::new(
                "key-1", "vk-1",
            )],
            router: ditto_server::gateway::RouterConfig {
                default_backends: vec![ditto_server::gateway::RouteBackend {
                    backend: "primary".to_string(),
                    weight: 1.0,
                }],
                rules: Vec::new(),
            },
            a2a_agents: Vec::new(),
            mcp_servers: Vec::new(),
            observability: Default::default(),
        };

        config
            .validate()
            .expect("startup validation should accept configured provider backends");
    }

    #[cfg(feature = "gateway-config-yaml")]
    #[test]
    fn load_gateway_config_imports_litellm_yaml() {
        let raw = r#"
model_list:
  - model_name: "*"
    litellm_params:
      model: "*"

general_settings:
  master_key: sk-1234
"#;
        let (_dir, path) = write_temp_file("gateway.yaml", raw);
        let config = load_gateway_config(test_locale(), path.to_str().expect("utf8 path"))
            .expect("load litellm yaml");
        assert_eq!(config.virtual_keys.len(), 1);
        assert_eq!(config.virtual_keys[0].token, "sk-1234");
        assert!(!config.backends.is_empty());
        assert!(!config.router.default_backends.is_empty());
    }
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!(
        "{}",
        cli_feature_disabled(
            MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US),
            "gateway",
            "--features gateway",
        )
    );
}
