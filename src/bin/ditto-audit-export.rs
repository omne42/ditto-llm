#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::StreamExt as _;
    use tokio::io::AsyncWriteExt as _;

    let mut args = std::env::args().skip(1);
    let usage = concat!(
        "usage: ditto-audit-export \\\n",
        "  --base-url URL \\\n",
        "  (--admin-token TOKEN | --admin-token-env ENV) \\\n",
        "  --output PATH|- \\\n",
        "  [--format jsonl|csv] [--limit N] [--since-ts-ms MS] [--before-ts-ms MS] \\\n",
        "  [--manifest-output PATH] \\\n",
        "  [--upload DEST] [--upload-manifest DEST] \\\n",
        "  [--s3-object-lock-mode MODE] [--s3-object-lock-retain-until-date RFC3339] [--s3-object-lock-legal-hold-status STATUS]\n",
        "\n",
        "DEST may be s3://bucket/key, gs://bucket/object, or http(s)://... (PUT).\n",
    );

    let mut base_url: Option<String> = None;
    let mut admin_token: Option<String> = None;
    let mut admin_token_env: Option<String> = None;
    let mut format: String = "jsonl".to_string();
    let mut limit: usize = 1000;
    let mut since_ts_ms: Option<u64> = None;
    let mut before_ts_ms: Option<u64> = None;
    let mut output: Option<String> = None;
    let mut manifest_output: Option<String> = None;
    let mut upload: Option<String> = None;
    let mut upload_manifest: Option<String> = None;

    let mut s3_object_lock_mode: Option<String> = None;
    let mut s3_object_lock_retain_until_date: Option<String> = None;
    let mut s3_object_lock_legal_hold_status: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base-url" => base_url = Some(args.next().ok_or("missing value for --base-url")?),
            "--admin-token" => {
                admin_token = Some(args.next().ok_or("missing value for --admin-token")?)
            }
            "--admin-token-env" => {
                admin_token_env = Some(args.next().ok_or("missing value for --admin-token-env")?)
            }
            "--format" => format = args.next().ok_or("missing value for --format")?,
            "--limit" => limit = args.next().ok_or("missing value for --limit")?.parse()?,
            "--since-ts-ms" => {
                since_ts_ms = Some(
                    args.next()
                        .ok_or("missing value for --since-ts-ms")?
                        .parse()?,
                )
            }
            "--before-ts-ms" => {
                before_ts_ms = Some(
                    args.next()
                        .ok_or("missing value for --before-ts-ms")?
                        .parse()?,
                )
            }
            "--output" => output = Some(args.next().ok_or("missing value for --output")?),
            "--manifest-output" => {
                manifest_output = Some(args.next().ok_or("missing value for --manifest-output")?)
            }
            "--upload" => upload = Some(args.next().ok_or("missing value for --upload")?),
            "--upload-manifest" => {
                upload_manifest = Some(args.next().ok_or("missing value for --upload-manifest")?)
            }
            "--s3-object-lock-mode" => {
                s3_object_lock_mode = Some(
                    args.next()
                        .ok_or("missing value for --s3-object-lock-mode")?,
                )
            }
            "--s3-object-lock-retain-until-date" => {
                s3_object_lock_retain_until_date = Some(
                    args.next()
                        .ok_or("missing value for --s3-object-lock-retain-until-date")?,
                )
            }
            "--s3-object-lock-legal-hold-status" => {
                s3_object_lock_legal_hold_status = Some(
                    args.next()
                        .ok_or("missing value for --s3-object-lock-legal-hold-status")?,
                )
            }
            "--help" | "-h" => {
                println!("{usage}");
                return Ok(());
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let base_url = base_url.ok_or(usage)?;
    let output = output.ok_or(usage)?;

    if output == "-" && upload.is_some() {
        return Err("cannot use --output - together with --upload".into());
    }

    let admin_token = admin_token
        .or_else(|| {
            admin_token_env.map(|env| match std::env::var(&env) {
                Ok(value) => value,
                Err(err) => format!("__ERROR__:{env}:{err}"),
            })
        })
        .ok_or("missing --admin-token or --admin-token-env")?;

    if let Some(error) = admin_token.strip_prefix("__ERROR__:") {
        return Err(format!("failed to read admin token from env: {error}").into());
    }

    let format = format.trim().to_ascii_lowercase();
    if format != "jsonl" && format != "ndjson" && format != "csv" {
        return Err(format!("unsupported format: {format}").into());
    }

    let base = base_url.trim_end_matches('/');
    let mut url = reqwest::Url::parse(&format!("{base}/admin/audit/export"))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("format", &format);
        qp.append_pair("limit", &limit.to_string());
        if let Some(value) = since_ts_ms {
            qp.append_pair("since_ts_ms", &value.to_string());
        }
        if let Some(value) = before_ts_ms {
            qp.append_pair("before_ts_ms", &value.to_string());
        }
    }

    let client = reqwest::Client::new();
    let response = client
        .get(url.clone())
        .header("authorization", format!("Bearer {admin_token}"))
        .send()
        .await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("export failed: HTTP {status} {body}").into());
    }

    let content_type = match format.as_str() {
        "csv" => "text/csv",
        _ => "application/x-ndjson",
    };

    if output == "-" {
        use std::io::Write as _;

        let mut stdout = std::io::stdout();
        let mut stream = response.bytes_stream();
        let mut bytes_written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            stdout.write_all(&chunk)?;
            bytes_written = bytes_written.saturating_add(chunk.len() as u64);
        }
        stdout.flush()?;
        eprintln!("wrote {bytes_written} bytes to stdout");
        return Ok(());
    }

    let mut file = tokio::fs::File::create(&output).await?;
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    let mut bytes_written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        bytes_written = bytes_written.saturating_add(chunk.len() as u64);
    }
    file.flush().await?;
    drop(file);

    let sha256_hex = hex_lower(&hasher.finalize());

    let (records, chain_last_hash) = if format == "jsonl" || format == "ndjson" {
        let (records, last) = verify_audit_export_jsonl(&output)?;
        (Some(records), last)
    } else {
        (None, None)
    };

    let manifest = serde_json::json!({
        "base_url": base,
        "export_url": url.as_str(),
        "format": format,
        "since_ts_ms": since_ts_ms,
        "before_ts_ms": before_ts_ms,
        "limit": limit,
        "content_type": content_type,
        "bytes": bytes_written,
        "sha256": sha256_hex,
        "records": records,
        "hash_chain_last": chain_last_hash,
        "generated_at_ms": now_ms(),
    });

    let manifest_output = manifest_output.unwrap_or_else(|| format!("{output}.manifest.json"));
    let serialized = serde_json::to_vec_pretty(&manifest)?;
    tokio::fs::write(&manifest_output, serialized).await?;

    if let Some(dest) = upload.as_deref() {
        upload_file(
            &client,
            &output,
            dest,
            content_type,
            &s3_object_lock_mode,
            &s3_object_lock_retain_until_date,
            &s3_object_lock_legal_hold_status,
        )
        .await?;

        let manifest_dest = upload_manifest.unwrap_or_else(|| format!("{dest}.manifest.json"));
        upload_file(
            &client,
            &manifest_output,
            &manifest_dest,
            "application/json",
            &s3_object_lock_mode,
            &s3_object_lock_retain_until_date,
            &s3_object_lock_legal_hold_status,
        )
        .await?;
    }

    println!("ok");
    eprintln!("output: {output}");
    eprintln!("manifest: {manifest_output}");
    Ok(())
}

#[cfg(feature = "gateway")]
fn now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

#[cfg(feature = "gateway")]
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(feature = "gateway")]
fn audit_chain_hash(
    prev_hash: Option<&str>,
    record: &ditto_llm::gateway::AuditLogRecord,
) -> String {
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
    if let Some(prev_hash) = prev_hash {
        hasher.update(prev_hash.as_bytes());
    }
    hasher.update(b"\n");
    if let Ok(serialized) = serde_json::to_vec(record) {
        hasher.update(&serialized);
    }
    hex_lower(&hasher.finalize())
}

#[cfg(feature = "gateway")]
fn verify_audit_export_jsonl(
    path: &str,
) -> Result<(usize, Option<String>), Box<dyn std::error::Error>> {
    use std::io::BufRead as _;

    #[derive(Debug, serde::Deserialize)]
    struct AuditExportRecord {
        id: i64,
        ts_ms: u64,
        kind: String,
        payload: serde_json::Value,
        #[serde(default)]
        prev_hash: Option<String>,
        hash: String,
    }

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let mut prev_hash: Option<String> = None;
    let mut count: usize = 0;
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: AuditExportRecord = serde_json::from_str(line)?;

        let expected_prev = prev_hash.as_deref().unwrap_or("");
        let got_prev = record.prev_hash.as_deref().unwrap_or("");
        if expected_prev != got_prev {
            return Err(format!(
                "hash chain mismatch at line {}: prev_hash expected {} got {}",
                line_no + 1,
                expected_prev,
                got_prev
            )
            .into());
        }

        let base = ditto_llm::gateway::AuditLogRecord {
            id: record.id,
            ts_ms: record.ts_ms,
            kind: record.kind,
            payload: record.payload,
        };
        let expected_hash = audit_chain_hash(prev_hash.as_deref(), &base);
        if record.hash != expected_hash {
            return Err(format!(
                "hash mismatch at line {}: expected {} got {}",
                line_no + 1,
                expected_hash,
                record.hash
            )
            .into());
        }
        prev_hash = Some(record.hash);
        count = count.saturating_add(1);
    }

    Ok((count, prev_hash))
}

#[cfg(feature = "gateway")]
async fn upload_file(
    client: &reqwest::Client,
    local_path: &str,
    dest: &str,
    content_type: &str,
    s3_object_lock_mode: &Option<String>,
    s3_object_lock_retain_until_date: &Option<String>,
    s3_object_lock_legal_hold_status: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((bucket, key)) = parse_s3_uri(dest) {
        return upload_to_s3_via_aws_cli(
            local_path,
            &bucket,
            &key,
            content_type,
            s3_object_lock_mode,
            s3_object_lock_retain_until_date,
            s3_object_lock_legal_hold_status,
        )
        .await;
    }

    if dest.starts_with("gs://") {
        return upload_to_gcs_via_gsutil(local_path, dest, content_type).await;
    }

    if dest.starts_with("http://") || dest.starts_with("https://") {
        return upload_to_http_put(client, local_path, dest, content_type).await;
    }

    Err(format!("unsupported upload destination: {dest}").into())
}

#[cfg(feature = "gateway")]
fn parse_s3_uri(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("s3://")?;
    let (bucket, key) = rest.split_once('/')?;
    let bucket = bucket.trim();
    let key = key.trim();
    if bucket.is_empty() || key.is_empty() {
        return None;
    }
    Some((bucket.to_string(), key.to_string()))
}

#[cfg(all(test, feature = "gateway"))]
mod tests {
    use super::parse_s3_uri;

    #[test]
    fn parse_s3_uri_rejects_missing_key() {
        assert!(parse_s3_uri("s3://bucket").is_none());
        assert!(parse_s3_uri("s3://bucket/").is_none());
        assert!(parse_s3_uri("s3:///key").is_none());
        assert!(parse_s3_uri("s3:// /key").is_none());
    }

    #[test]
    fn parse_s3_uri_accepts_bucket_and_key() {
        assert_eq!(
            parse_s3_uri("s3://my-bucket/path/to/file.jsonl"),
            Some(("my-bucket".to_string(), "path/to/file.jsonl".to_string()))
        );
    }
}

#[cfg(feature = "gateway")]
async fn upload_to_s3_via_aws_cli(
    local_path: &str,
    bucket: &str,
    key: &str,
    content_type: &str,
    object_lock_mode: &Option<String>,
    object_lock_retain_until_date: &Option<String>,
    object_lock_legal_hold_status: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = tokio::process::Command::new("aws");
    cmd.arg("s3api")
        .arg("put-object")
        .arg("--bucket")
        .arg(bucket)
        .arg("--key")
        .arg(key)
        .arg("--body")
        .arg(local_path)
        .arg("--content-type")
        .arg(content_type);

    if let Some(value) = object_lock_mode.as_deref() {
        cmd.arg("--object-lock-mode").arg(value);
    }
    if let Some(value) = object_lock_retain_until_date.as_deref() {
        cmd.arg("--object-lock-retain-until-date").arg(value);
    }
    if let Some(value) = object_lock_legal_hold_status.as_deref() {
        cmd.arg("--object-lock-legal-hold-status").arg(value);
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(format!(
            "aws s3api put-object failed (exit={}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

#[cfg(feature = "gateway")]
async fn upload_to_gcs_via_gsutil(
    local_path: &str,
    dest: &str,
    content_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = tokio::process::Command::new("gsutil");
    cmd.arg("-h")
        .arg(format!("Content-Type:{content_type}"))
        .arg("cp")
        .arg(local_path)
        .arg(dest);

    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(format!(
            "gsutil cp failed (exit={}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

#[cfg(feature = "gateway")]
async fn upload_to_http_put(
    client: &reqwest::Client,
    local_path: &str,
    dest: &str,
    content_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = tokio::fs::File::open(local_path).await?;
    let len = file.metadata().await?.len();
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = reqwest::Body::wrap_stream(stream);

    let response = client
        .put(dest)
        .header("content-type", content_type)
        .header("content-length", len)
        .body(body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("http upload failed: HTTP {status} {body}").into());
    }
    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("ditto-audit-export requires `--features gateway`");
    std::process::exit(2);
}
