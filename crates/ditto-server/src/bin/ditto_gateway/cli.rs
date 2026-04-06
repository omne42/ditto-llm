#[cfg(feature = "gateway")]
use std::path::PathBuf;

#[cfg(feature = "gateway")]
use ditto_core::resources::MESSAGE_CATALOG;
#[cfg(feature = "gateway")]
use i18n_kit::{Locale, TemplateArg};

#[cfg(feature = "gateway")]
#[derive(Debug)]
pub(crate) struct GatewayCliArgs {
    pub path: String,
    pub listen: String,
    pub admin_token: Option<String>,
    pub admin_token_env: Option<String>,
    pub admin_read_token: Option<String>,
    pub admin_read_token_env: Option<String>,
    pub admin_tenant_tokens: Vec<(String, String)>,
    pub admin_tenant_token_env: Vec<(String, String)>,
    pub admin_tenant_read_tokens: Vec<(String, String)>,
    pub admin_tenant_read_token_env: Vec<(String, String)>,
    pub dotenv_path: Option<PathBuf>,
    pub state_path: Option<PathBuf>,
    pub sqlite_path: Option<PathBuf>,
    pub postgres_url: Option<String>,
    pub postgres_url_env: Option<String>,
    pub mysql_url: Option<String>,
    pub mysql_url_env: Option<String>,
    pub redis_url: Option<String>,
    pub redis_url_env: Option<String>,
    pub redis_prefix: Option<String>,
    pub audit_retention_secs: Option<u64>,
    pub db_doctor: bool,
    pub backend_specs: Vec<String>,
    pub upstream_specs: Vec<String>,
    pub json_logs: bool,
    pub proxy_cache_enabled: bool,
    pub proxy_cache_ttl_seconds: Option<u64>,
    pub proxy_cache_max_entries: Option<usize>,
    pub proxy_cache_max_body_bytes: Option<usize>,
    pub proxy_cache_max_total_body_bytes: Option<usize>,
    pub proxy_cache_streaming_enabled: bool,
    pub proxy_cache_max_stream_body_bytes: Option<usize>,
    pub proxy_max_body_bytes: Option<usize>,
    pub proxy_usage_max_body_bytes: Option<usize>,
    pub proxy_max_in_flight: Option<usize>,
    pub pricing_litellm_path: Option<String>,
    pub prometheus_metrics_enabled: bool,
    pub prometheus_max_key_series: Option<usize>,
    pub prometheus_max_model_series: Option<usize>,
    pub prometheus_max_backend_series: Option<usize>,
    pub prometheus_max_path_series: Option<usize>,
    pub proxy_retry_enabled: bool,
    pub proxy_retry_status_codes: Option<Vec<u16>>,
    pub proxy_fallback_status_codes: Option<Vec<u16>>,
    pub proxy_network_error_action: Option<String>,
    pub proxy_timeout_error_action: Option<String>,
    pub proxy_retry_max_attempts: Option<usize>,
    pub proxy_circuit_breaker_enabled: bool,
    pub proxy_cb_failure_threshold: Option<u32>,
    pub proxy_cb_cooldown_secs: Option<u64>,
    pub proxy_cb_failure_status_codes: Option<Vec<u16>>,
    pub proxy_cb_no_network_errors: bool,
    pub proxy_cb_no_timeout_errors: bool,
    pub proxy_cb_no_server_errors: bool,
    pub proxy_health_checks_enabled: bool,
    pub proxy_health_check_path: Option<String>,
    pub proxy_health_check_interval_secs: Option<u64>,
    pub proxy_health_check_timeout_secs: Option<u64>,
    pub devtools_path: Option<String>,
    pub otel_enabled: bool,
    pub otel_endpoint: Option<String>,
    pub otel_json: bool,
}

#[cfg(feature = "gateway")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_gateway_cli_args(
    args: impl Iterator<Item = String>,
) -> Result<GatewayCliArgs, Box<dyn std::error::Error>> {
    parse_gateway_cli_args_with_locale(
        args,
        MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US),
    )
}

#[cfg(feature = "gateway")]
pub(crate) fn parse_gateway_cli_args_with_locale(
    mut args: impl Iterator<Item = String>,
    locale: Locale,
) -> Result<GatewayCliArgs, Box<dyn std::error::Error>> {
    let usage = gateway_cli_usage(locale);
    let path = match args.next() {
        Some(flag) if flag == "--help" || flag == "-h" => return Err(usage.into()),
        Some(path) => path,
        None => return Err(usage.into()),
    };

    let mut listen = "127.0.0.1:8080".to_string();
    let mut admin_token: Option<String> = None;
    let mut admin_token_env: Option<String> = None;
    let mut admin_read_token: Option<String> = None;
    let mut admin_read_token_env: Option<String> = None;
    let mut admin_tenant_tokens: Vec<(String, String)> = Vec::new();
    let mut admin_tenant_token_env: Vec<(String, String)> = Vec::new();
    let mut admin_tenant_read_tokens: Vec<(String, String)> = Vec::new();
    let mut admin_tenant_read_token_env: Vec<(String, String)> = Vec::new();
    let mut dotenv_path: Option<PathBuf> = None;
    let mut state_path: Option<PathBuf> = None;
    let mut sqlite_path: Option<PathBuf> = None;
    let mut postgres_url: Option<String> = None;
    let mut postgres_url_env: Option<String> = None;
    let mut mysql_url: Option<String> = None;
    let mut mysql_url_env: Option<String> = None;
    let mut redis_url: Option<String> = None;
    let mut redis_url_env: Option<String> = None;
    let mut redis_prefix: Option<String> = None;
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let mut audit_retention_secs: Option<u64> = None;
    #[cfg(not(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    )))]
    let audit_retention_secs: Option<u64> = None;
    let mut db_doctor = false;
    let mut backend_specs: Vec<String> = Vec::new();
    let mut upstream_specs: Vec<String> = Vec::new();
    let mut json_logs = false;
    let mut proxy_cache_enabled = false;
    let mut proxy_cache_ttl_seconds: Option<u64> = None;
    let mut proxy_cache_max_entries: Option<usize> = None;
    let mut proxy_cache_max_body_bytes: Option<usize> = None;
    let mut proxy_cache_max_total_body_bytes: Option<usize> = None;
    let mut proxy_cache_streaming_enabled = false;
    let mut proxy_cache_max_stream_body_bytes: Option<usize> = None;
    let mut proxy_max_body_bytes: Option<usize> = None;
    let mut proxy_usage_max_body_bytes: Option<usize> = None;
    let mut proxy_max_in_flight: Option<usize> = None;
    let mut pricing_litellm_path: Option<String> = None;
    let mut prometheus_metrics_enabled = false;
    let mut prometheus_max_key_series: Option<usize> = None;
    let mut prometheus_max_model_series: Option<usize> = None;
    let mut prometheus_max_backend_series: Option<usize> = None;
    let mut prometheus_max_path_series: Option<usize> = None;
    let mut proxy_retry_enabled = false;
    let mut proxy_retry_status_codes: Option<Vec<u16>> = None;
    let mut proxy_fallback_status_codes: Option<Vec<u16>> = None;
    let mut proxy_network_error_action: Option<String> = None;
    let mut proxy_timeout_error_action: Option<String> = None;
    let mut proxy_retry_max_attempts: Option<usize> = None;
    let mut proxy_circuit_breaker_enabled = false;
    let mut proxy_cb_failure_threshold: Option<u32> = None;
    let mut proxy_cb_cooldown_secs: Option<u64> = None;
    let mut proxy_cb_failure_status_codes: Option<Vec<u16>> = None;
    let mut proxy_cb_no_network_errors = false;
    let mut proxy_cb_no_timeout_errors = false;
    let mut proxy_cb_no_server_errors = false;
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
            "--dotenv" => {
                dotenv_path = Some(next_value(&mut args, locale, "--dotenv")?.into());
            }
            "--listen" | "--addr" => {
                listen = next_value(&mut args, locale, "--listen/--addr")?;
            }
            "--admin-token" => {
                admin_token = Some(next_value(&mut args, locale, "--admin-token")?);
            }
            "--admin-token-env" => {
                admin_token_env = Some(next_value(&mut args, locale, "--admin-token-env")?);
            }
            "--admin-read-token" => {
                admin_read_token = Some(next_value(&mut args, locale, "--admin-read-token")?);
            }
            "--admin-read-token-env" => {
                admin_read_token_env =
                    Some(next_value(&mut args, locale, "--admin-read-token-env")?);
            }
            "--admin-tenant-token" => {
                let spec = next_value(&mut args, locale, "--admin-tenant-token")?;
                let (tenant_id, token) = spec.split_once('=').ok_or_else(|| {
                    invalid_spec(locale, "--admin-tenant-token", "TENANT_ID=TOKEN")
                })?;
                admin_tenant_tokens.push((tenant_id.to_string(), token.to_string()));
            }
            "--admin-tenant-token-env" => {
                let spec = next_value(&mut args, locale, "--admin-tenant-token-env")?;
                let (tenant_id, env_key) = spec.split_once('=').ok_or_else(|| {
                    invalid_spec(locale, "--admin-tenant-token-env", "TENANT_ID=ENV")
                })?;
                admin_tenant_token_env.push((tenant_id.to_string(), env_key.to_string()));
            }
            "--admin-tenant-read-token" => {
                let spec = next_value(&mut args, locale, "--admin-tenant-read-token")?;
                let (tenant_id, token) = spec.split_once('=').ok_or_else(|| {
                    invalid_spec(locale, "--admin-tenant-read-token", "TENANT_ID=TOKEN")
                })?;
                admin_tenant_read_tokens.push((tenant_id.to_string(), token.to_string()));
            }
            "--admin-tenant-read-token-env" => {
                let spec = next_value(&mut args, locale, "--admin-tenant-read-token-env")?;
                let (tenant_id, env_key) = spec.split_once('=').ok_or_else(|| {
                    invalid_spec(locale, "--admin-tenant-read-token-env", "TENANT_ID=ENV")
                })?;
                admin_tenant_read_token_env.push((tenant_id.to_string(), env_key.to_string()));
            }
            "--state" => {
                state_path = Some(next_value(&mut args, locale, "--state")?.into());
            }
            "--sqlite" => {
                sqlite_path = Some(next_value(&mut args, locale, "--sqlite")?.into());
            }
            "--pg" | "--postgres" => {
                postgres_url = Some(next_value(&mut args, locale, "--pg/--postgres")?);
            }
            "--pg-env" | "--postgres-env" => {
                postgres_url_env = Some(next_value(&mut args, locale, "--pg-env/--postgres-env")?);
            }
            "--mysql" => {
                mysql_url = Some(next_value(&mut args, locale, "--mysql")?);
            }
            "--mysql-env" => {
                mysql_url_env = Some(next_value(&mut args, locale, "--mysql-env")?);
            }
            "--redis" => {
                redis_url = Some(next_value(&mut args, locale, "--redis")?);
            }
            "--redis-env" => {
                redis_url_env = Some(next_value(&mut args, locale, "--redis-env")?);
            }
            "--redis-prefix" => {
                redis_prefix = Some(next_value(&mut args, locale, "--redis-prefix")?);
            }
            "--audit-retention-secs" => {
                #[cfg(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                ))]
                {
                    let secs = parse_next::<u64>(&mut args, locale, "--audit-retention-secs")?;
                    audit_retention_secs = Some(secs);
                }

                #[cfg(not(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-postgres",
                    feature = "gateway-store-mysql",
                    feature = "gateway-store-redis"
                )))]
                {
                    return Err(MESSAGE_CATALOG.render(
                        locale,
                        "cli.requires_feature",
                        &[
                            TemplateArg::new("flag", "--audit-retention-secs"),
                            TemplateArg::new(
                                "feature",
                                "gateway-store-sqlite | gateway-store-postgres | gateway-store-mysql | gateway-store-redis",
                            ),
                        ],
                    )
                    .into());
                }
            }
            "--db-doctor" => {
                db_doctor = true;
            }
            "--backend" => {
                backend_specs.push(next_value(&mut args, locale, "--backend")?);
            }
            "--upstream" => {
                upstream_specs.push(next_value(&mut args, locale, "--upstream")?);
            }
            "--json-logs" => {
                json_logs = true;
            }
            "--proxy-cache" => {
                proxy_cache_enabled = true;
            }
            "--proxy-cache-ttl" => {
                proxy_cache_enabled = true;
                proxy_cache_ttl_seconds =
                    Some(parse_next::<u64>(&mut args, locale, "--proxy-cache-ttl")?);
            }
            "--proxy-cache-max-entries" => {
                proxy_cache_enabled = true;
                proxy_cache_max_entries = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-cache-max-entries",
                )?);
            }
            "--proxy-cache-max-body-bytes" => {
                proxy_cache_enabled = true;
                proxy_cache_max_body_bytes = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-cache-max-body-bytes",
                )?);
            }
            "--proxy-cache-max-total-body-bytes" => {
                proxy_cache_enabled = true;
                proxy_cache_max_total_body_bytes = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-cache-max-total-body-bytes",
                )?);
            }
            "--proxy-cache-streaming" => {
                proxy_cache_enabled = true;
                proxy_cache_streaming_enabled = true;
            }
            "--proxy-cache-max-stream-body-bytes" => {
                proxy_cache_enabled = true;
                proxy_cache_max_stream_body_bytes = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-cache-max-stream-body-bytes",
                )?);
            }
            "--proxy-max-in-flight" => {
                proxy_max_in_flight = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-max-in-flight",
                )?);
            }
            "--proxy-max-body-bytes" => {
                proxy_max_body_bytes = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-max-body-bytes",
                )?);
            }
            "--proxy-usage-max-body-bytes" => {
                proxy_usage_max_body_bytes = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-usage-max-body-bytes",
                )?);
            }
            "--pricing-litellm" => {
                pricing_litellm_path = Some(next_value(&mut args, locale, "--pricing-litellm")?);
            }
            "--prometheus-metrics" => {
                prometheus_metrics_enabled = true;
            }
            "--prometheus-max-key-series" => {
                prometheus_metrics_enabled = true;
                prometheus_max_key_series = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--prometheus-max-key-series",
                )?);
            }
            "--prometheus-max-model-series" => {
                prometheus_metrics_enabled = true;
                prometheus_max_model_series = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--prometheus-max-model-series",
                )?);
            }
            "--prometheus-max-backend-series" => {
                prometheus_metrics_enabled = true;
                prometheus_max_backend_series = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--prometheus-max-backend-series",
                )?);
            }
            "--prometheus-max-path-series" => {
                prometheus_metrics_enabled = true;
                prometheus_max_path_series = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--prometheus-max-path-series",
                )?);
            }
            "--proxy-retry" => {
                proxy_retry_enabled = true;
            }
            "--proxy-retry-status-codes" => {
                proxy_retry_enabled = true;
                proxy_retry_status_codes = Some(parse_status_codes(
                    &next_value(&mut args, locale, "--proxy-retry-status-codes")?,
                    locale,
                )?);
            }
            "--proxy-fallback-status-codes" => {
                proxy_fallback_status_codes = Some(parse_status_codes(
                    &next_value(&mut args, locale, "--proxy-fallback-status-codes")?,
                    locale,
                )?);
            }
            "--proxy-network-error-action" => {
                proxy_network_error_action = Some(parse_proxy_failure_action(
                    &next_value(&mut args, locale, "--proxy-network-error-action")?,
                    locale,
                )?);
            }
            "--proxy-timeout-error-action" => {
                proxy_timeout_error_action = Some(parse_proxy_failure_action(
                    &next_value(&mut args, locale, "--proxy-timeout-error-action")?,
                    locale,
                )?);
            }
            "--proxy-retry-max-attempts" => {
                proxy_retry_enabled = true;
                proxy_retry_max_attempts = Some(parse_next::<usize>(
                    &mut args,
                    locale,
                    "--proxy-retry-max-attempts",
                )?);
            }
            "--proxy-circuit-breaker" => {
                proxy_circuit_breaker_enabled = true;
            }
            "--proxy-cb-failure-threshold" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_failure_threshold = Some(parse_next::<u32>(
                    &mut args,
                    locale,
                    "--proxy-cb-failure-threshold",
                )?);
            }
            "--proxy-cb-cooldown-secs" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_cooldown_secs = Some(parse_next::<u64>(
                    &mut args,
                    locale,
                    "--proxy-cb-cooldown-secs",
                )?);
            }
            "--proxy-cb-failure-status-codes" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_failure_status_codes = Some(parse_status_codes(
                    &next_value(&mut args, locale, "--proxy-cb-failure-status-codes")?,
                    locale,
                )?);
            }
            "--proxy-cb-no-network-errors" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_no_network_errors = true;
            }
            "--proxy-cb-no-timeout-errors" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_no_timeout_errors = true;
            }
            "--proxy-cb-no-server-errors" => {
                proxy_circuit_breaker_enabled = true;
                proxy_cb_no_server_errors = true;
            }
            "--proxy-health-checks" => {
                proxy_health_checks_enabled = true;
            }
            "--proxy-health-check-path" => {
                proxy_health_checks_enabled = true;
                proxy_health_check_path =
                    Some(next_value(&mut args, locale, "--proxy-health-check-path")?);
            }
            "--proxy-health-check-interval-secs" => {
                proxy_health_checks_enabled = true;
                proxy_health_check_interval_secs = Some(parse_next::<u64>(
                    &mut args,
                    locale,
                    "--proxy-health-check-interval-secs",
                )?);
            }
            "--proxy-health-check-timeout-secs" => {
                proxy_health_checks_enabled = true;
                proxy_health_check_timeout_secs = Some(parse_next::<u64>(
                    &mut args,
                    locale,
                    "--proxy-health-check-timeout-secs",
                )?);
            }
            "--devtools" => {
                devtools_path = Some(next_value(&mut args, locale, "--devtools")?);
            }
            "--otel" => {
                otel_enabled = true;
            }
            "--otel-endpoint" => {
                otel_enabled = true;
                otel_endpoint = Some(next_value(&mut args, locale, "--otel-endpoint")?);
            }
            "--otel-json" => {
                otel_enabled = true;
                otel_json = true;
            }
            other => return Err(unknown_arg(locale, other, &usage)),
        }
    }

    validate_single_control_plane_persistence_target(
        locale,
        state_path.is_some(),
        sqlite_path.is_some(),
        postgres_url.is_some() || postgres_url_env.is_some(),
        mysql_url.is_some() || mysql_url_env.is_some(),
        redis_url.is_some() || redis_url_env.is_some(),
    )?;

    Ok(GatewayCliArgs {
        path,
        listen,
        admin_token,
        admin_token_env,
        admin_read_token,
        admin_read_token_env,
        admin_tenant_tokens,
        admin_tenant_token_env,
        admin_tenant_read_tokens,
        admin_tenant_read_token_env,
        dotenv_path,
        state_path,
        sqlite_path,
        postgres_url,
        postgres_url_env,
        mysql_url,
        mysql_url_env,
        redis_url,
        redis_url_env,
        redis_prefix,
        audit_retention_secs,
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
    })
}

#[cfg(feature = "gateway")]
fn parse_proxy_failure_action(
    raw: &str,
    locale: Locale,
) -> Result<String, Box<dyn std::error::Error>> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "none" | "fallback" | "retry" => Ok(normalized),
        _ => Err(invalid_value(
            locale,
            "--proxy-network-error-action/--proxy-timeout-error-action",
        )),
    }
}

#[cfg(feature = "gateway")]
fn parse_status_codes(raw: &str, locale: Locale) -> Result<Vec<u16>, Box<dyn std::error::Error>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(empty_status_code_list(locale));
    }

    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(
            part.parse::<u16>()
                .map_err(|_| invalid_status_code(locale))?,
        );
    }
    if out.is_empty() {
        return Err(empty_status_code_list(locale));
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(feature = "gateway")]
pub(crate) async fn resolve_cli_secret(
    raw: String,
    env: &ditto_core::config::Env,
    label: &str,
    locale: Locale,
) -> Result<String, Box<dyn std::error::Error>> {
    let raw = raw.trim().to_string();
    if !raw.starts_with("secret://") {
        return Ok(raw);
    }

    let resolved = secret_kit::spec::resolve_secret(raw.as_str(), env)
        .await
        .map(|secret| secret.into_owned())
        .map_err(|err| {
            MESSAGE_CATALOG.render(
                locale,
                "cli.failed_to_resolve",
                &[
                    TemplateArg::new("label", label),
                    TemplateArg::new("error", err.to_string()),
                ],
            )
        })?;
    if resolved.trim().is_empty() {
        return Err(MESSAGE_CATALOG
            .render(
                locale,
                "cli.resolved_empty",
                &[TemplateArg::new("label", label)],
            )
            .into());
    }
    Ok(resolved)
}

#[cfg(feature = "gateway")]
pub(crate) fn gateway_cli_usage(locale: Locale) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.usage",
        &[TemplateArg::new("command_and_syntax", usage_syntax())],
    )
}

#[cfg(feature = "gateway")]
fn usage_syntax() -> &'static str {
    #[cfg(feature = "gateway-config-yaml")]
    {
        "ditto-gateway [config.(json|yaml)] [--dotenv PATH] [--listen|--addr HOST:PORT] [--admin-token TOKEN] [--admin-token-env ENV] [--admin-read-token TOKEN] [--admin-read-token-env ENV] [--admin-tenant-token TENANT=TOKEN] [--admin-tenant-token-env TENANT=ENV] [--admin-tenant-read-token TENANT=TOKEN] [--admin-tenant-read-token-env TENANT=ENV] [--state PATH] [--sqlite PATH] [--pg URL] [--pg-env ENV] [--mysql URL] [--mysql-env ENV] [--redis URL] [--redis-env ENV] [--redis-prefix PREFIX] [--audit-retention-secs SECS] [--db-doctor] [--backend name=url] [--upstream name=base_url] [--json-logs] [--proxy-cache] [--proxy-cache-ttl SECS] [--proxy-cache-max-entries N] [--proxy-cache-max-body-bytes N] [--proxy-cache-max-total-body-bytes N] [--proxy-cache-streaming] [--proxy-cache-max-stream-body-bytes N] [--proxy-max-body-bytes N] [--proxy-usage-max-body-bytes N] [--proxy-max-in-flight N] [--proxy-retry] [--proxy-retry-status-codes CODES] [--proxy-fallback-status-codes CODES] [--proxy-network-error-action ACTION] [--proxy-timeout-error-action ACTION] [--proxy-retry-max-attempts N] [--proxy-circuit-breaker] [--proxy-cb-failure-threshold N] [--proxy-cb-cooldown-secs SECS] [--proxy-cb-failure-status-codes CODES] [--proxy-cb-no-network-errors] [--proxy-cb-no-timeout-errors] [--proxy-cb-no-server-errors] [--proxy-health-checks] [--proxy-health-check-path PATH] [--proxy-health-check-interval-secs SECS] [--proxy-health-check-timeout-secs SECS] [--pricing-litellm PATH] [--prometheus-metrics] [--prometheus-max-key-series N] [--prometheus-max-model-series N] [--prometheus-max-backend-series N] [--prometheus-max-path-series N] [--devtools PATH] [--otel] [--otel-endpoint URL] [--otel-json]"
    }
    #[cfg(not(feature = "gateway-config-yaml"))]
    {
        "ditto-gateway [config.json] [--dotenv PATH] [--listen|--addr HOST:PORT] [--admin-token TOKEN] [--admin-token-env ENV] [--admin-read-token TOKEN] [--admin-read-token-env ENV] [--admin-tenant-token TENANT=TOKEN] [--admin-tenant-token-env TENANT=ENV] [--admin-tenant-read-token TENANT=TOKEN] [--admin-tenant-read-token-env TENANT=ENV] [--state PATH] [--sqlite PATH] [--pg URL] [--pg-env ENV] [--mysql URL] [--mysql-env ENV] [--redis URL] [--redis-env ENV] [--redis-prefix PREFIX] [--audit-retention-secs SECS] [--db-doctor] [--backend name=url] [--upstream name=base_url] [--json-logs] [--proxy-cache] [--proxy-cache-ttl SECS] [--proxy-cache-max-entries N] [--proxy-cache-max-body-bytes N] [--proxy-cache-max-total-body-bytes N] [--proxy-cache-streaming] [--proxy-cache-max-stream-body-bytes N] [--proxy-max-body-bytes N] [--proxy-usage-max-body-bytes N] [--proxy-max-in-flight N] [--proxy-retry] [--proxy-retry-status-codes CODES] [--proxy-fallback-status-codes CODES] [--proxy-network-error-action ACTION] [--proxy-timeout-error-action ACTION] [--proxy-retry-max-attempts N] [--proxy-circuit-breaker] [--proxy-cb-failure-threshold N] [--proxy-cb-cooldown-secs SECS] [--proxy-cb-failure-status-codes CODES] [--proxy-cb-no-network-errors] [--proxy-cb-no-timeout-errors] [--proxy-cb-no-server-errors] [--proxy-health-checks] [--proxy-health-check-path PATH] [--proxy-health-check-interval-secs SECS] [--proxy-health-check-timeout-secs SECS] [--pricing-litellm PATH] [--prometheus-metrics] [--prometheus-max-key-series N] [--prometheus-max-model-series N] [--prometheus-max-backend-series N] [--prometheus-max-path-series N] [--devtools PATH] [--otel] [--otel-endpoint URL] [--otel-json]"
    }
}

#[cfg(feature = "gateway")]
fn next_value(
    args: &mut impl Iterator<Item = String>,
    locale: Locale,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next().ok_or_else(|| {
        MESSAGE_CATALOG
            .render(
                locale,
                "cli.missing_value",
                &[TemplateArg::new("flag", flag)],
            )
            .into()
    })
}

#[cfg(feature = "gateway")]
fn parse_next<T>(
    args: &mut impl Iterator<Item = String>,
    locale: Locale,
    flag: &str,
) -> Result<T, Box<dyn std::error::Error>>
where
    T: std::str::FromStr,
{
    let raw = next_value(args, locale, flag)?;
    raw.parse::<T>().map_err(|_| invalid_value(locale, flag))
}

#[cfg(feature = "gateway")]
fn invalid_value(locale: Locale, flag: &str) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.invalid_value",
            &[TemplateArg::new("label", flag)],
        )
        .into()
}

#[cfg(feature = "gateway")]
fn invalid_spec(locale: Locale, flag: &str, expected: &str) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(
            locale,
            "cli.invalid_spec",
            &[
                TemplateArg::new("label", flag),
                TemplateArg::new("expected", expected),
            ],
        )
        .into()
}

#[cfg(feature = "gateway")]
fn validate_single_control_plane_persistence_target(
    locale: Locale,
    has_state: bool,
    has_sqlite: bool,
    has_postgres: bool,
    has_mysql: bool,
    has_redis: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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
fn empty_status_code_list(locale: Locale) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(locale, "cli.empty_status_code_list", &[])
        .into()
}

#[cfg(feature = "gateway")]
fn invalid_status_code(locale: Locale) -> Box<dyn std::error::Error> {
    MESSAGE_CATALOG
        .render(locale, "cli.invalid_status_code", &[])
        .into()
}

#[cfg(feature = "gateway")]
fn unknown_arg(locale: Locale, arg: &str, usage: &str) -> Box<dyn std::error::Error> {
    let message =
        MESSAGE_CATALOG.render(locale, "cli.unknown_arg", &[TemplateArg::new("arg", arg)]);
    format!("{message}\n{usage}").into()
}

#[cfg(all(test, feature = "gateway"))]
mod tests {
    use super::*;

    fn runtime_catalog_available() -> bool {
        MESSAGE_CATALOG.with_catalog(|_| ()).is_ok()
    }

    #[test]
    fn parses_minimal_args() {
        let cli =
            parse_gateway_cli_args(vec!["gateway.json".to_string()].into_iter()).expect("parse");
        assert_eq!(cli.path, "gateway.json");
        assert_eq!(cli.listen, "127.0.0.1:8080");
        assert!(!cli.json_logs);
    }

    #[test]
    fn proxy_cache_ttl_enables_cache() {
        let cli = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--proxy-cache-ttl".to_string(),
                "10".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        assert!(cli.proxy_cache_enabled);
        assert_eq!(cli.proxy_cache_ttl_seconds, Some(10));
    }

    #[test]
    fn proxy_cache_streaming_flags_parse() {
        let cli = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--proxy-cache-streaming".to_string(),
                "--proxy-cache-max-stream-body-bytes".to_string(),
                "2048".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        assert!(cli.proxy_cache_enabled);
        assert!(cli.proxy_cache_streaming_enabled);
        assert_eq!(cli.proxy_cache_max_stream_body_bytes, Some(2048));
    }

    #[test]
    fn addr_alias_sets_listen() {
        let cli = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--addr".to_string(),
                "0.0.0.0:9999".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        assert_eq!(cli.listen, "0.0.0.0:9999");
    }

    #[test]
    fn rejects_multiple_control_plane_persistence_targets() {
        let err = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--state".to_string(),
                "./gateway-state.json".to_string(),
                "--sqlite".to_string(),
                "./gateway.sqlite".to_string(),
                "--pg".to_string(),
                "postgres://user:pass@localhost/ditto".to_string(),
            ]
            .into_iter(),
        )
        .expect_err("multiple persistence targets should be rejected");
        assert!(err.to_string().contains(
            "choose exactly one of --state, --sqlite, --pg/--postgres, --mysql, or --redis"
        ));
    }

    #[test]
    fn rejects_unknown_args() {
        let err = parse_gateway_cli_args(
            vec!["gateway.json".to_string(), "--wat".to_string()].into_iter(),
        )
        .expect_err("reject");
        if !runtime_catalog_available() {
            return;
        }
        assert!(err.to_string().contains("unknown arg"));
    }

    #[test]
    fn parses_db_doctor_flag() {
        let cli = parse_gateway_cli_args(
            vec!["gateway.json".to_string(), "--db-doctor".to_string()].into_iter(),
        )
        .expect("parse");
        assert!(cli.db_doctor);
    }

    #[test]
    fn parses_proxy_fallback_status_codes() {
        let cli = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--proxy-fallback-status-codes".to_string(),
                "500, 502 ,500".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        assert_eq!(cli.proxy_fallback_status_codes, Some(vec![500, 502]));
        assert!(!cli.proxy_retry_enabled);
    }

    #[test]
    fn parses_proxy_transport_and_circuit_breaker_failure_flags() {
        let cli = parse_gateway_cli_args(
            vec![
                "gateway.json".to_string(),
                "--proxy-network-error-action".to_string(),
                "retry".to_string(),
                "--proxy-timeout-error-action".to_string(),
                "none".to_string(),
                "--proxy-cb-failure-status-codes".to_string(),
                "408,429".to_string(),
                "--proxy-cb-no-network-errors".to_string(),
                "--proxy-cb-no-timeout-errors".to_string(),
                "--proxy-circuit-breaker".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse");
        assert_eq!(cli.proxy_network_error_action.as_deref(), Some("retry"));
        assert_eq!(cli.proxy_timeout_error_action.as_deref(), Some("none"));
        assert_eq!(cli.proxy_cb_failure_status_codes, Some(vec![408, 429]));
        assert!(cli.proxy_cb_no_network_errors);
        assert!(cli.proxy_cb_no_timeout_errors);
        assert!(cli.proxy_circuit_breaker_enabled);
    }
}
