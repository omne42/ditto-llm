#[cfg(feature = "gateway")]
use std::path::PathBuf;

#[cfg(all(
    feature = "gateway",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql"
    )
))]
use std::time::Instant;

use ditto_core::resources::{MESSAGE_CATALOG, bootstrap_cli_runtime_from_args_with_defaults};
use i18n_kit::{Locale, TemplateArg};
#[cfg(feature = "gateway")]
use serde::Serialize;

#[cfg(feature = "gateway")]
#[derive(Debug)]
struct BenchArgs {
    sqlite_path: Option<PathBuf>,
    postgres_url: Option<String>,
    mysql_url: Option<String>,
    audit_ops: usize,
    reap_ops: usize,
    out_path: Option<PathBuf>,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Serialize)]
struct BenchReport {
    generated_at_ms: u64,
    audit_ops: usize,
    reap_ops: usize,
    results: Vec<StoreBenchResult>,
}

#[cfg(feature = "gateway")]
#[derive(Debug, Serialize)]
struct StoreBenchResult {
    store: String,
    audit_append_ms: u128,
    audit_append_ops_per_sec: f64,
    audit_cleanup_ms: u128,
    audit_cleanup_deleted: u64,
    reap_ms: u128,
    budget_scanned: u64,
    budget_reaped: u64,
    budget_released: u64,
    cost_scanned: u64,
    cost_reaped: u64,
    cost_released: u64,
}

#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Err(err) = bootstrap_cli_runtime_from_args_with_defaults(
        &raw_args,
        ditto_server::data_root::default_server_data_root_files(),
    ) {
        eprintln!("{err:?}");
        std::process::exit(2);
    }
    let (locale, args) = match MESSAGE_CATALOG.resolve_cli_locale(raw_args, "DITTO_LOCALE") {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    if let Err(err) = run(locale, args).await {
        eprintln!("{}", render_error(err.as_ref(), locale));
        std::process::exit(1);
    }
}

#[cfg(feature = "gateway")]
async fn run(locale: Locale, raw_args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    if raw_args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage(locale));
        return Ok(());
    }

    let args = parse_args(raw_args.into_iter(), locale)?;

    #[allow(unused_mut)]
    let mut results = Vec::new();

    if let Some(_path) = args.sqlite_path.as_ref() {
        #[cfg(feature = "gateway-store-sqlite")]
        {
            let result = run_sqlite(_path.clone(), args.audit_ops, args.reap_ops).await?;
            results.push(result);
        }
        #[cfg(not(feature = "gateway-store-sqlite"))]
        {
            return Err(MESSAGE_CATALOG
                .render(
                    locale,
                    "cli.requires_feature",
                    &[
                        TemplateArg::new("flag", "--sqlite"),
                        TemplateArg::new("feature", "gateway-store-sqlite"),
                    ],
                )
                .into());
        }
    }

    if let Some(_url) = args.postgres_url.as_ref() {
        #[cfg(feature = "gateway-store-postgres")]
        {
            let result = run_postgres(_url, args.audit_ops, args.reap_ops).await?;
            results.push(result);
        }
        #[cfg(not(feature = "gateway-store-postgres"))]
        {
            return Err(MESSAGE_CATALOG
                .render(
                    locale,
                    "cli.requires_feature",
                    &[
                        TemplateArg::new("flag", "--pg"),
                        TemplateArg::new("feature", "gateway-store-postgres"),
                    ],
                )
                .into());
        }
    }

    if let Some(_url) = args.mysql_url.as_ref() {
        #[cfg(feature = "gateway-store-mysql")]
        {
            let result = run_mysql(_url, args.audit_ops, args.reap_ops).await?;
            results.push(result);
        }
        #[cfg(not(feature = "gateway-store-mysql"))]
        {
            return Err(MESSAGE_CATALOG
                .render(
                    locale,
                    "cli.requires_feature",
                    &[
                        TemplateArg::new("flag", "--mysql"),
                        TemplateArg::new("feature", "gateway-store-mysql"),
                    ],
                )
                .into());
        }
    }

    if results.is_empty() {
        return Err(no_stores_selected(locale).into());
    }

    let report = BenchReport {
        generated_at_ms: now_millis_u64(),
        audit_ops: args.audit_ops,
        reap_ops: args.reap_ops,
        results,
    };

    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");

    if let Some(path) = args.out_path {
        std::fs::write(path, json)?;
    }

    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!(
        "{}",
        MESSAGE_CATALOG.render(
            MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US),
            "cli.feature_disabled",
            &[
                TemplateArg::new("feature", "gateway"),
                TemplateArg::new("rebuild_hint", "--features gateway"),
            ],
        )
    );
}

#[cfg(feature = "gateway")]
fn parse_args(
    mut args: impl Iterator<Item = String>,
    locale: Locale,
) -> Result<BenchArgs, Box<dyn std::error::Error>> {
    let usage = usage(locale);
    let mut sqlite_path: Option<PathBuf> = None;
    let mut postgres_url: Option<String> = None;
    let mut mysql_url: Option<String> = None;
    let mut audit_ops: usize = 5_000;
    let mut reap_ops: usize = 2_000;
    let mut out_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sqlite" => {
                sqlite_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            MESSAGE_CATALOG.render(
                                locale,
                                "cli.missing_value",
                                &[TemplateArg::new("flag", "--sqlite")],
                            )
                        })?
                        .into(),
                );
            }
            "--pg" => {
                postgres_url = Some(args.next().ok_or_else(|| {
                    MESSAGE_CATALOG.render(
                        locale,
                        "cli.missing_value",
                        &[TemplateArg::new("flag", "--pg")],
                    )
                })?);
            }
            "--mysql" => {
                mysql_url = Some(args.next().ok_or_else(|| {
                    MESSAGE_CATALOG.render(
                        locale,
                        "cli.missing_value",
                        &[TemplateArg::new("flag", "--mysql")],
                    )
                })?);
            }
            "--audit-ops" => {
                audit_ops = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[TemplateArg::new("flag", "--audit-ops")],
                        )
                    })?
                    .parse::<usize>()
                    .map_err(|_| invalid_value(locale, "--audit-ops"))?;
            }
            "--reap-ops" => {
                reap_ops = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[TemplateArg::new("flag", "--reap-ops")],
                        )
                    })?
                    .parse::<usize>()
                    .map_err(|_| invalid_value(locale, "--reap-ops"))?;
            }
            "--out" => {
                out_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            MESSAGE_CATALOG.render(
                                locale,
                                "cli.missing_value",
                                &[TemplateArg::new("flag", "--out")],
                            )
                        })?
                        .into(),
                );
            }
            "--help" | "-h" => {
                return Err(usage.into());
            }
            other => {
                let message = MESSAGE_CATALOG.render(
                    locale,
                    "cli.unknown_arg",
                    &[TemplateArg::new("arg", other)],
                );
                return Err(format!("{message}\n{usage}").into());
            }
        }
    }

    if audit_ops == 0 {
        return Err(must_be_positive(locale, "--audit-ops").into());
    }
    if reap_ops == 0 {
        return Err(must_be_positive(locale, "--reap-ops").into());
    }

    Ok(BenchArgs {
        sqlite_path,
        postgres_url,
        mysql_url,
        audit_ops,
        reap_ops,
        out_path,
    })
}

#[cfg(feature = "gateway")]
fn usage(locale: Locale) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.usage",
        &[TemplateArg::new(
            "command_and_syntax",
            "ditto-store-bench [--sqlite PATH] [--pg URL] [--mysql URL] [--audit-ops N] [--reap-ops N] [--out PATH]",
        )],
    )
}

#[cfg(feature = "gateway")]
fn invalid_value(locale: Locale, flag: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.invalid_value",
        &[TemplateArg::new("label", flag)],
    )
}

#[cfg(feature = "gateway")]
fn must_be_positive(locale: Locale, flag: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.must_be_positive",
        &[TemplateArg::new("flag", flag)],
    )
}

#[cfg(feature = "gateway")]
fn no_stores_selected(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "store_bench.no_stores_selected", &[])
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

#[cfg(all(feature = "gateway", feature = "gateway-store-sqlite"))]
async fn run_sqlite(
    path: PathBuf,
    audit_ops: usize,
    reap_ops: usize,
) -> Result<StoreBenchResult, Box<dyn std::error::Error>> {
    use ditto_server::gateway::SqliteStore;

    let store = SqliteStore::new(path).with_audit_retention_secs(Some(1));
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "sqlite",
        StoreWorkloadSize {
            audit_ops,
            reap_ops,
        },
        StoreWorkloadOps {
            reserve_budget: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_budget_tokens(&request_id, &key_id, limit, value)
                        .await
                }
            },
            reserve_cost: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                        .await
                }
            },
            append_audit: |kind, payload| {
                let store = store.clone();
                async move { store.append_audit_log(kind, payload).await }
            },
            reap_audit: |cutoff_ts_ms| {
                let store = store.clone();
                async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
            },
            reap_budget: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_budget_reservations(u64::MAX, limit, false)
                        .await
                }
            },
            reap_cost: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_cost_reservations(u64::MAX, limit, false)
                        .await
                }
            },
        },
    )
    .await
}

#[cfg(all(feature = "gateway", feature = "gateway-store-postgres"))]
async fn run_postgres(
    url: &str,
    audit_ops: usize,
    reap_ops: usize,
) -> Result<StoreBenchResult, Box<dyn std::error::Error>> {
    use ditto_server::gateway::PostgresStore;

    let store = PostgresStore::connect(url)
        .await?
        .with_audit_retention_secs(Some(1));
    store.ping().await?;
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "postgres",
        StoreWorkloadSize {
            audit_ops,
            reap_ops,
        },
        StoreWorkloadOps {
            reserve_budget: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_budget_tokens(&request_id, &key_id, limit, value)
                        .await
                }
            },
            reserve_cost: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                        .await
                }
            },
            append_audit: |kind, payload| {
                let store = store.clone();
                async move { store.append_audit_log(kind, payload).await }
            },
            reap_audit: |cutoff_ts_ms| {
                let store = store.clone();
                async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
            },
            reap_budget: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_budget_reservations(u64::MAX, limit, false)
                        .await
                }
            },
            reap_cost: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_cost_reservations(u64::MAX, limit, false)
                        .await
                }
            },
        },
    )
    .await
}

#[cfg(all(feature = "gateway", feature = "gateway-store-mysql"))]
async fn run_mysql(
    url: &str,
    audit_ops: usize,
    reap_ops: usize,
) -> Result<StoreBenchResult, Box<dyn std::error::Error>> {
    use ditto_server::gateway::MySqlStore;

    let store = MySqlStore::connect(url)
        .await?
        .with_audit_retention_secs(Some(1));
    store.ping().await?;
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "mysql",
        StoreWorkloadSize {
            audit_ops,
            reap_ops,
        },
        StoreWorkloadOps {
            reserve_budget: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_budget_tokens(&request_id, &key_id, limit, value)
                        .await
                }
            },
            reserve_cost: |request_id: String, key_id: String, limit, value| {
                let store = store.clone();
                async move {
                    store
                        .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                        .await
                }
            },
            append_audit: |kind, payload| {
                let store = store.clone();
                async move { store.append_audit_log(kind, payload).await }
            },
            reap_audit: |cutoff_ts_ms| {
                let store = store.clone();
                async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
            },
            reap_budget: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_budget_reservations(u64::MAX, limit, false)
                        .await
                }
            },
            reap_cost: |limit| {
                let store = store.clone();
                async move {
                    store
                        .reap_stale_cost_reservations(u64::MAX, limit, false)
                        .await
                }
            },
        },
    )
    .await
}

#[cfg(feature = "gateway")]
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
))]
struct StoreWorkloadSize {
    audit_ops: usize,
    reap_ops: usize,
}

#[cfg(feature = "gateway")]
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
))]
struct StoreWorkloadOps<FB, FC, FA, FRA, FRB, FRC> {
    reserve_budget: FB,
    reserve_cost: FC,
    append_audit: FA,
    reap_audit: FRA,
    reap_budget: FRB,
    reap_cost: FRC,
}

#[cfg(feature = "gateway")]
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
))]
async fn run_store_workload<
    FB,
    FC,
    FA,
    FRA,
    FRB,
    FRC,
    FutB,
    FutC,
    FutA,
    FutRA,
    FutRB,
    FutRC,
    BE,
    CE,
    AE,
    RAE,
    RBE,
    RCE,
>(
    store_name: &str,
    workload: StoreWorkloadSize,
    ops: StoreWorkloadOps<FB, FC, FA, FRA, FRB, FRC>,
) -> Result<StoreBenchResult, Box<dyn std::error::Error>>
where
    FB: Fn(String, String, u64, u64) -> FutB,
    FC: Fn(String, String, u64, u64) -> FutC,
    FA: Fn(String, serde_json::Value) -> FutA,
    FRA: Fn(u64) -> FutRA,
    FRB: Fn(usize) -> FutRB,
    FRC: Fn(usize) -> FutRC,
    FutB: std::future::Future<Output = Result<(), BE>>,
    FutC: std::future::Future<Output = Result<(), CE>>,
    FutA: std::future::Future<Output = Result<(), AE>>,
    FutRA: std::future::Future<Output = Result<u64, RAE>>,
    FutRB: std::future::Future<Output = Result<(u64, u64, u64), RBE>>,
    FutRC: std::future::Future<Output = Result<(u64, u64, u64), RCE>>,
    BE: std::error::Error + Send + Sync + 'static,
    CE: std::error::Error + Send + Sync + 'static,
    AE: std::error::Error + Send + Sync + 'static,
    RAE: std::error::Error + Send + Sync + 'static,
    RBE: std::error::Error + Send + Sync + 'static,
    RCE: std::error::Error + Send + Sync + 'static,
{
    let StoreWorkloadSize {
        audit_ops,
        reap_ops,
    } = workload;
    let StoreWorkloadOps {
        reserve_budget,
        reserve_cost,
        append_audit,
        reap_audit,
        reap_budget,
        reap_cost,
    } = ops;

    let base_ms = now_millis_u64();
    let key_id = format!("bench-key-{store_name}-{base_ms}");
    let limit = u64::try_from(reap_ops)
        .unwrap_or(u64::MAX / 4)
        .saturating_mul(10)
        .saturating_add(10_000);

    let t0 = Instant::now();
    for i in 0..audit_ops {
        append_audit(
            format!("bench.audit.{store_name}"),
            serde_json::json!({"i": i, "store": store_name}),
        )
        .await
        .map_err(|err| format!("{store_name} append_audit_log failed: {err}"))?;
    }
    let audit_elapsed = t0.elapsed();

    let t_cleanup = Instant::now();
    let audit_cleanup_deleted = reap_audit(u64::MAX)
        .await
        .map_err(|err| format!("{store_name} reap_audit_logs_before failed: {err}"))?;
    let audit_cleanup_elapsed = t_cleanup.elapsed();

    for i in 0..reap_ops {
        reserve_budget(
            format!("bench-budget-{store_name}-{base_ms}-{i}"),
            key_id.clone(),
            limit,
            1,
        )
        .await
        .map_err(|err| format!("{store_name} reserve_budget_tokens failed: {err}"))?;

        reserve_cost(
            format!("bench-cost-{store_name}-{base_ms}-{i}"),
            key_id.clone(),
            limit,
            1,
        )
        .await
        .map_err(|err| format!("{store_name} reserve_cost_usd_micros failed: {err}"))?;
    }

    let t1 = Instant::now();
    let (budget_scanned, budget_reaped, budget_released) = reap_budget(reap_ops.saturating_mul(2))
        .await
        .map_err(|err| format!("{store_name} reap_stale_budget_reservations failed: {err}"))?;
    let (cost_scanned, cost_reaped, cost_released) = reap_cost(reap_ops.saturating_mul(2))
        .await
        .map_err(|err| format!("{store_name} reap_stale_cost_reservations failed: {err}"))?;
    let reap_elapsed = t1.elapsed();

    let audit_ops_per_sec = (audit_ops as f64) / audit_elapsed.as_secs_f64();

    Ok(StoreBenchResult {
        store: store_name.to_string(),
        audit_append_ms: audit_elapsed.as_millis(),
        audit_append_ops_per_sec: if audit_ops_per_sec.is_finite() {
            audit_ops_per_sec
        } else {
            0.0
        },
        audit_cleanup_ms: audit_cleanup_elapsed.as_millis(),
        audit_cleanup_deleted,
        reap_ms: reap_elapsed.as_millis(),
        budget_scanned,
        budget_reaped,
        budget_released,
        cost_scanned,
        cost_reaped,
        cost_released,
    })
}

#[cfg(feature = "gateway")]
fn now_millis_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
