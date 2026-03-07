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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(std::env::args().skip(1))?;

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
            return Err("--sqlite requires `--features gateway-store-sqlite`".into());
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
            return Err("--pg requires `--features gateway-store-postgres`".into());
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
            return Err("--mysql requires `--features gateway-store-mysql`".into());
        }
    }

    if results.is_empty() {
        return Err("no stores selected; pass --sqlite and/or --pg and/or --mysql".into());
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
    eprintln!("gateway feature disabled; rebuild with --features gateway");
}

#[cfg(feature = "gateway")]
fn parse_args(
    mut args: impl Iterator<Item = String>,
) -> Result<BenchArgs, Box<dyn std::error::Error>> {
    let mut sqlite_path: Option<PathBuf> = None;
    let mut postgres_url: Option<String> = None;
    let mut mysql_url: Option<String> = None;
    let mut audit_ops: usize = 5_000;
    let mut reap_ops: usize = 2_000;
    let mut out_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sqlite" => {
                sqlite_path = Some(args.next().ok_or("missing value for --sqlite")?.into());
            }
            "--pg" => {
                postgres_url = Some(args.next().ok_or("missing value for --pg")?);
            }
            "--mysql" => {
                mysql_url = Some(args.next().ok_or("missing value for --mysql")?);
            }
            "--audit-ops" => {
                audit_ops = args
                    .next()
                    .ok_or("missing value for --audit-ops")?
                    .parse::<usize>()
                    .map_err(|_| "invalid --audit-ops")?;
            }
            "--reap-ops" => {
                reap_ops = args
                    .next()
                    .ok_or("missing value for --reap-ops")?
                    .parse::<usize>()
                    .map_err(|_| "invalid --reap-ops")?;
            }
            "--out" => {
                out_path = Some(args.next().ok_or("missing value for --out")?.into());
            }
            "--help" | "-h" => {
                return Err(usage().into());
            }
            other => {
                return Err(format!("unknown arg: {other}\n{}", usage()).into());
            }
        }
    }

    if audit_ops == 0 {
        return Err("--audit-ops must be > 0".into());
    }
    if reap_ops == 0 {
        return Err("--reap-ops must be > 0".into());
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
fn usage() -> &'static str {
    "usage: ditto-store-bench [--sqlite PATH] [--pg URL] [--mysql URL] [--audit-ops N] [--reap-ops N] [--out PATH]"
}

#[cfg(all(feature = "gateway", feature = "gateway-store-sqlite"))]
async fn run_sqlite(
    path: PathBuf,
    audit_ops: usize,
    reap_ops: usize,
) -> Result<StoreBenchResult, Box<dyn std::error::Error>> {
    use ditto_llm::gateway::SqliteStore;

    let store = SqliteStore::new(path).with_audit_retention_secs(Some(1));
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "sqlite",
        audit_ops,
        reap_ops,
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_budget_tokens(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |kind, payload| {
            let store = store.clone();
            async move { store.append_audit_log(kind, payload).await }
        },
        |cutoff_ts_ms| {
            let store = store.clone();
            async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_budget_reservations(u64::MAX, limit, false)
                    .await
            }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_cost_reservations(u64::MAX, limit, false)
                    .await
            }
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
    use ditto_llm::gateway::PostgresStore;

    let store = PostgresStore::connect(url)
        .await?
        .with_audit_retention_secs(Some(1));
    store.ping().await?;
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "postgres",
        audit_ops,
        reap_ops,
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_budget_tokens(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |kind, payload| {
            let store = store.clone();
            async move { store.append_audit_log(kind, payload).await }
        },
        |cutoff_ts_ms| {
            let store = store.clone();
            async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_budget_reservations(u64::MAX, limit, false)
                    .await
            }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_cost_reservations(u64::MAX, limit, false)
                    .await
            }
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
    use ditto_llm::gateway::MySqlStore;

    let store = MySqlStore::connect(url)
        .await?
        .with_audit_retention_secs(Some(1));
    store.ping().await?;
    store.init().await?;
    store.verify_schema().await?;

    run_store_workload(
        "mysql",
        audit_ops,
        reap_ops,
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_budget_tokens(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |request_id, key_id, limit, value| {
            let store = store.clone();
            async move {
                store
                    .reserve_cost_usd_micros(&request_id, &key_id, limit, value)
                    .await
            }
        },
        |kind, payload| {
            let store = store.clone();
            async move { store.append_audit_log(kind, payload).await }
        },
        |cutoff_ts_ms| {
            let store = store.clone();
            async move { store.reap_audit_logs_before(cutoff_ts_ms).await }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_budget_reservations(u64::MAX, limit, false)
                    .await
            }
        },
        |limit| {
            let store = store.clone();
            async move {
                store
                    .reap_stale_cost_reservations(u64::MAX, limit, false)
                    .await
            }
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
    audit_ops: usize,
    reap_ops: usize,
    reserve_budget: FB,
    reserve_cost: FC,
    append_audit: FA,
    reap_audit: FRA,
    reap_budget: FRB,
    reap_cost: FRC,
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
