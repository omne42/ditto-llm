#[cfg(feature = "gateway")]
use std::path::PathBuf;

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
    pub redis_url: Option<String>,
    pub redis_url_env: Option<String>,
    pub redis_prefix: Option<String>,
    pub audit_retention_secs: Option<u64>,
    pub backend_specs: Vec<String>,
    pub upstream_specs: Vec<String>,
    pub json_logs: bool,
    pub proxy_cache_enabled: bool,
    pub proxy_cache_ttl_seconds: Option<u64>,
    pub proxy_cache_max_entries: Option<usize>,
    pub proxy_cache_max_body_bytes: Option<usize>,
    pub proxy_cache_max_total_body_bytes: Option<usize>,
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
    pub proxy_retry_max_attempts: Option<usize>,
    pub proxy_circuit_breaker_enabled: bool,
    pub proxy_cb_failure_threshold: Option<u32>,
    pub proxy_cb_cooldown_secs: Option<u64>,
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
pub(crate) fn parse_gateway_cli_args(
    mut args: impl Iterator<Item = String>,
) -> Result<GatewayCliArgs, Box<dyn std::error::Error>> {
    let usage = usage();
    let path = args.next().ok_or(usage)?;

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
    let mut redis_url: Option<String> = None;
    let mut redis_url_env: Option<String> = None;
    let mut redis_prefix: Option<String> = None;
    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let mut audit_retention_secs: Option<u64> = None;
    #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
    let audit_retention_secs: Option<u64> = None;
    let mut backend_specs: Vec<String> = Vec::new();
    let mut upstream_specs: Vec<String> = Vec::new();
    let mut json_logs = false;
    let mut proxy_cache_enabled = false;
    let mut proxy_cache_ttl_seconds: Option<u64> = None;
    let mut proxy_cache_max_entries: Option<usize> = None;
    let mut proxy_cache_max_body_bytes: Option<usize> = None;
    let mut proxy_cache_max_total_body_bytes: Option<usize> = None;
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
            "--dotenv" => {
                dotenv_path = Some(args.next().ok_or("missing value for --dotenv")?.into());
            }
            "--listen" | "--addr" => {
                listen = args.next().ok_or("missing value for --listen/--addr")?;
            }
            "--admin-token" => {
                admin_token = Some(args.next().ok_or("missing value for --admin-token")?);
            }
            "--admin-token-env" => {
                admin_token_env = Some(args.next().ok_or("missing value for --admin-token-env")?);
            }
            "--admin-read-token" => {
                admin_read_token = Some(args.next().ok_or("missing value for --admin-read-token")?);
            }
            "--admin-read-token-env" => {
                admin_read_token_env = Some(
                    args.next()
                        .ok_or("missing value for --admin-read-token-env")?,
                );
            }
            "--admin-tenant-token" => {
                let spec = args
                    .next()
                    .ok_or("missing value for --admin-tenant-token")?;
                let (tenant_id, token) = spec
                    .split_once('=')
                    .ok_or("--admin-tenant-token must be TENANT_ID=TOKEN")?;
                admin_tenant_tokens.push((tenant_id.to_string(), token.to_string()));
            }
            "--admin-tenant-token-env" => {
                let spec = args
                    .next()
                    .ok_or("missing value for --admin-tenant-token-env")?;
                let (tenant_id, env_key) = spec
                    .split_once('=')
                    .ok_or("--admin-tenant-token-env must be TENANT_ID=ENV")?;
                admin_tenant_token_env.push((tenant_id.to_string(), env_key.to_string()));
            }
            "--admin-tenant-read-token" => {
                let spec = args
                    .next()
                    .ok_or("missing value for --admin-tenant-read-token")?;
                let (tenant_id, token) = spec
                    .split_once('=')
                    .ok_or("--admin-tenant-read-token must be TENANT_ID=TOKEN")?;
                admin_tenant_read_tokens.push((tenant_id.to_string(), token.to_string()));
            }
            "--admin-tenant-read-token-env" => {
                let spec = args
                    .next()
                    .ok_or("missing value for --admin-tenant-read-token-env")?;
                let (tenant_id, env_key) = spec
                    .split_once('=')
                    .ok_or("--admin-tenant-read-token-env must be TENANT_ID=ENV")?;
                admin_tenant_read_token_env.push((tenant_id.to_string(), env_key.to_string()));
            }
            "--state" => {
                state_path = Some(args.next().ok_or("missing value for --state")?.into());
            }
            "--sqlite" => {
                sqlite_path = Some(args.next().ok_or("missing value for --sqlite")?.into());
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
            "--audit-retention-secs" => {
                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
                {
                    let raw = args
                        .next()
                        .ok_or("missing value for --audit-retention-secs")?;
                    let secs = raw
                        .parse::<u64>()
                        .map_err(|_| "invalid --audit-retention-secs")?;
                    audit_retention_secs = Some(secs);
                }

                #[cfg(not(any(
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-redis"
                )))]
                {
                    return Err(
                        "--audit-retention-secs requires `--features gateway-store-sqlite` or `--features gateway-store-redis`"
                            .into(),
                    );
                }
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
            "--proxy-max-body-bytes" => {
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-max-body-bytes")?;
                proxy_max_body_bytes = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-max-body-bytes")?,
                );
            }
            "--proxy-usage-max-body-bytes" => {
                let raw = args
                    .next()
                    .ok_or("missing value for --proxy-usage-max-body-bytes")?;
                proxy_usage_max_body_bytes = Some(
                    raw.parse::<usize>()
                        .map_err(|_| "invalid --proxy-usage-max-body-bytes")?,
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
        redis_url,
        redis_url_env,
        redis_prefix,
        audit_retention_secs,
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
    })
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

#[cfg(feature = "gateway")]
pub(crate) async fn resolve_cli_secret(
    raw: String,
    env: &ditto_llm::Env,
    label: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let raw = raw.trim().to_string();
    if !raw.starts_with("secret://") {
        return Ok(raw);
    }

    let resolved = ditto_llm::secrets::resolve_secret_string(raw.as_str(), env)
        .await
        .map_err(|err| format!("failed to resolve {label}: {err}"))?;
    if resolved.trim().is_empty() {
        return Err(format!("{label} resolved to an empty value").into());
    }
    Ok(resolved)
}

#[cfg(feature = "gateway")]
fn usage() -> &'static str {
    #[cfg(feature = "gateway-config-yaml")]
    {
        "usage: ditto-gateway <config.(json|yaml)> [--dotenv PATH] [--listen|--addr HOST:PORT] [--admin-token TOKEN] [--admin-token-env ENV] [--admin-read-token TOKEN] [--admin-read-token-env ENV] [--admin-tenant-token TENANT=TOKEN] [--admin-tenant-token-env TENANT=ENV] [--admin-tenant-read-token TENANT=TOKEN] [--admin-tenant-read-token-env TENANT=ENV] [--state PATH] [--sqlite PATH] [--redis URL] [--redis-env ENV] [--redis-prefix PREFIX] [--audit-retention-secs SECS] [--backend name=url] [--upstream name=base_url] [--json-logs] [--proxy-cache] [--proxy-cache-ttl SECS] [--proxy-cache-max-entries N] [--proxy-cache-max-body-bytes N] [--proxy-cache-max-total-body-bytes N] [--proxy-max-body-bytes N] [--proxy-usage-max-body-bytes N] [--proxy-max-in-flight N] [--proxy-retry] [--proxy-retry-status-codes CODES] [--proxy-retry-max-attempts N] [--proxy-circuit-breaker] [--proxy-cb-failure-threshold N] [--proxy-cb-cooldown-secs SECS] [--proxy-health-checks] [--proxy-health-check-path PATH] [--proxy-health-check-interval-secs SECS] [--proxy-health-check-timeout-secs SECS] [--pricing-litellm PATH] [--prometheus-metrics] [--prometheus-max-key-series N] [--prometheus-max-model-series N] [--prometheus-max-backend-series N] [--prometheus-max-path-series N] [--devtools PATH] [--otel] [--otel-endpoint URL] [--otel-json]"
    }
    #[cfg(not(feature = "gateway-config-yaml"))]
    {
        "usage: ditto-gateway <config.json> [--dotenv PATH] [--listen|--addr HOST:PORT] [--admin-token TOKEN] [--admin-token-env ENV] [--admin-read-token TOKEN] [--admin-read-token-env ENV] [--admin-tenant-token TENANT=TOKEN] [--admin-tenant-token-env TENANT=ENV] [--admin-tenant-read-token TENANT=TOKEN] [--admin-tenant-read-token-env TENANT=ENV] [--state PATH] [--sqlite PATH] [--redis URL] [--redis-env ENV] [--redis-prefix PREFIX] [--audit-retention-secs SECS] [--backend name=url] [--upstream name=base_url] [--json-logs] [--proxy-cache] [--proxy-cache-ttl SECS] [--proxy-cache-max-entries N] [--proxy-cache-max-body-bytes N] [--proxy-cache-max-total-body-bytes N] [--proxy-max-body-bytes N] [--proxy-usage-max-body-bytes N] [--proxy-max-in-flight N] [--proxy-retry] [--proxy-retry-status-codes CODES] [--proxy-retry-max-attempts N] [--proxy-circuit-breaker] [--proxy-cb-failure-threshold N] [--proxy-cb-cooldown-secs SECS] [--proxy-health-checks] [--proxy-health-check-path PATH] [--proxy-health-check-interval-secs SECS] [--proxy-health-check-timeout-secs SECS] [--pricing-litellm PATH] [--prometheus-metrics] [--prometheus-max-key-series N] [--prometheus-max-model-series N] [--prometheus-max-backend-series N] [--prometheus-max-path-series N] [--devtools PATH] [--otel] [--otel-endpoint URL] [--otel-json]"
    }
}

#[cfg(all(test, feature = "gateway"))]
mod tests {
    use super::*;

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
    fn rejects_unknown_args() {
        let err = parse_gateway_cli_args(
            vec!["gateway.json".to_string(), "--wat".to_string()].into_iter(),
        )
        .expect_err("reject");
        assert!(err.to_string().contains("unknown arg"));
    }
}
