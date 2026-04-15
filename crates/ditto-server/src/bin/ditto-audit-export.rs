use ditto_core::resources::MESSAGE_CATALOG;
use i18n_kit::{Locale, TemplateArg};
#[cfg(feature = "gateway")]
use omne_integrity_primitives::Sha256Hasher;

#[cfg(feature = "gateway")]
#[derive(Clone, Copy)]
struct UploadOptions<'a> {
    content_type: &'a str,
    s3_object_lock_mode: Option<&'a str>,
    s3_object_lock_retain_until_date: Option<&'a str>,
    s3_object_lock_legal_hold_status: Option<&'a str>,
}

#[cfg(feature = "gateway")]
impl<'a> UploadOptions<'a> {
    fn new(
        content_type: &'a str,
        s3_object_lock_mode: &'a Option<String>,
        s3_object_lock_retain_until_date: &'a Option<String>,
        s3_object_lock_legal_hold_status: &'a Option<String>,
    ) -> Self {
        Self {
            content_type,
            s3_object_lock_mode: s3_object_lock_mode.as_deref(),
            s3_object_lock_retain_until_date: s3_object_lock_retain_until_date.as_deref(),
            s3_object_lock_legal_hold_status: s3_object_lock_legal_hold_status.as_deref(),
        }
    }
}

#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Err(err) = ditto_server::data_root::bootstrap_cli_runtime_from_args(&raw_args) {
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
    use futures_util::StreamExt as _;
    use tokio::io::AsyncWriteExt as _;

    let usage = audit_export_usage(locale);
    let mut args = raw_args.into_iter();

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
            "--base-url" => {
                base_url = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--base-url"))?,
                )
            }
            "--admin-token" => {
                admin_token = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--admin-token"))?,
                )
            }
            "--admin-token-env" => {
                admin_token_env = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--admin-token-env"))?,
                )
            }
            "--format" => {
                format = args
                    .next()
                    .ok_or_else(|| cli_missing_value(locale, "--format"))?
            }
            "--limit" => {
                limit = args
                    .next()
                    .ok_or_else(|| cli_missing_value(locale, "--limit"))?
                    .parse()
                    .map_err(|_| cli_invalid_value(locale, "--limit"))?
            }
            "--since-ts-ms" => {
                since_ts_ms = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--since-ts-ms"))?
                        .parse()
                        .map_err(|_| cli_invalid_value(locale, "--since-ts-ms"))?,
                )
            }
            "--before-ts-ms" => {
                before_ts_ms = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--before-ts-ms"))?
                        .parse()
                        .map_err(|_| cli_invalid_value(locale, "--before-ts-ms"))?,
                )
            }
            "--output" => {
                output = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--output"))?,
                )
            }
            "--manifest-output" => {
                manifest_output = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--manifest-output"))?,
                )
            }
            "--upload" => {
                upload = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--upload"))?,
                )
            }
            "--upload-manifest" => {
                upload_manifest = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--upload-manifest"))?,
                )
            }
            "--s3-object-lock-mode" => {
                s3_object_lock_mode = Some(
                    args.next()
                        .ok_or_else(|| cli_missing_value(locale, "--s3-object-lock-mode"))?,
                )
            }
            "--s3-object-lock-retain-until-date" => {
                s3_object_lock_retain_until_date = Some(args.next().ok_or_else(|| {
                    cli_missing_value(locale, "--s3-object-lock-retain-until-date")
                })?)
            }
            "--s3-object-lock-legal-hold-status" => {
                s3_object_lock_legal_hold_status = Some(args.next().ok_or_else(|| {
                    cli_missing_value(locale, "--s3-object-lock-legal-hold-status")
                })?)
            }
            "--help" | "-h" => {
                println!("{usage}");
                return Ok(());
            }
            other => {
                return Err(cli_unknown_arg(locale, other, Some(&usage)).into());
            }
        }
    }

    let base_url = base_url.ok_or_else(|| usage.clone())?;
    let output = output.ok_or_else(|| usage.clone())?;

    if output == "-" && upload.is_some() {
        return Err(audit_export_cannot_combine_output_upload(locale).into());
    }

    let admin_token = admin_token
        .or_else(|| {
            admin_token_env.map(|env| match std::env::var(&env) {
                Ok(value) => value,
                Err(err) => format!("__ERROR__:{env}:{err}"),
            })
        })
        .ok_or_else(|| audit_export_missing_admin_token(locale))?;

    if let Some(error) = admin_token.strip_prefix("__ERROR__:") {
        return Err(audit_export_failed_to_read_admin_token(locale, error).into());
    }

    let format = format.trim().to_ascii_lowercase();
    if format != "jsonl" && format != "ndjson" && format != "csv" {
        return Err(audit_export_unsupported_format(locale, &format).into());
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
        let body = http_kit::read_text_body_limited(response, 64 * 1024)
            .await
            .unwrap_or_default();
        return Err(audit_export_export_failed(locale, &status.to_string(), &body).into());
    }

    let content_type = match format.as_str() {
        "csv" => "text/csv",
        _ => "application/x-ndjson",
    };

    if output == "-" {
        use std::io::Write as _;

        struct CountingWrite<W> {
            inner: W,
            written: u64,
        }

        impl<W: std::io::Write> std::io::Write for CountingWrite<W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let written = self.inner.write(buf)?;
                self.written = self.written.saturating_add(written as u64);
                Ok(written)
            }

            fn flush(&mut self) -> std::io::Result<()> {
                self.inner.flush()
            }
        }

        let mut stdout = CountingWrite {
            inner: std::io::stdout(),
            written: 0,
        };
        http_kit::write_response_body_limited(response, &mut stdout, None).await?;
        stdout.flush()?;
        eprintln!("{}", cli_wrote_bytes_to_stdout(locale, stdout.written));
        return Ok(());
    }

    let mut file = tokio::fs::File::create(&output).await?;
    let mut hasher = Sha256Hasher::new();
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

    let sha256_hex = hasher.finalize().to_string();

    let (records, chain_last_hash) = if format == "jsonl" || format == "ndjson" {
        let (records, last) = verify_audit_export_jsonl(locale, &output)?;
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
        let upload_options = UploadOptions::new(
            content_type,
            &s3_object_lock_mode,
            &s3_object_lock_retain_until_date,
            &s3_object_lock_legal_hold_status,
        );
        upload_file(locale, &client, &output, dest, upload_options).await?;

        let manifest_dest = upload_manifest.unwrap_or_else(|| format!("{dest}.manifest.json"));
        upload_file(
            locale,
            &client,
            &manifest_output,
            &manifest_dest,
            UploadOptions {
                content_type: "application/json",
                ..upload_options
            },
        )
        .await?;
    }

    println!("{}", cli_ok(locale));
    eprintln!("{}", cli_output_path(locale, &output));
    eprintln!("{}", cli_manifest_path(locale, &manifest_output));
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
fn verify_audit_export_jsonl(
    locale: Locale,
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
            return Err(
                audit_hash_chain_mismatch(locale, line_no + 1, expected_prev, got_prev).into(),
            );
        }

        let base = ditto_server::gateway::AuditLogRecord {
            id: record.id,
            ts_ms: record.ts_ms,
            kind: record.kind,
            payload: record.payload,
        };
        let expected_hash =
            ditto_server::audit_integrity::audit_chain_hash(prev_hash.as_deref(), &base);
        if record.hash != expected_hash {
            return Err(
                audit_hash_mismatch(locale, line_no + 1, &expected_hash, &record.hash).into(),
            );
        }
        prev_hash = Some(record.hash);
        count = count.saturating_add(1);
    }

    Ok((count, prev_hash))
}

#[cfg(feature = "gateway")]
async fn upload_file(
    locale: Locale,
    client: &reqwest::Client,
    local_path: &str,
    dest: &str,
    options: UploadOptions<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((bucket, key)) = parse_s3_uri(dest) {
        return upload_to_s3_via_aws_cli(locale, local_path, &bucket, &key, options).await;
    }

    if dest.starts_with("gs://") {
        return upload_to_gcs_via_gsutil(locale, local_path, dest, options.content_type).await;
    }

    if dest.starts_with("http://") || dest.starts_with("https://") {
        return upload_to_http_put(locale, client, local_path, dest, options.content_type).await;
    }

    Err(audit_export_unsupported_upload_destination(locale, dest).into())
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
    locale: Locale,
    local_path: &str,
    bucket: &str,
    key: &str,
    options: UploadOptions<'_>,
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
        .arg(options.content_type);

    if let Some(value) = options.s3_object_lock_mode {
        cmd.arg("--object-lock-mode").arg(value);
    }
    if let Some(value) = options.s3_object_lock_retain_until_date {
        cmd.arg("--object-lock-retain-until-date").arg(value);
    }
    if let Some(value) = options.s3_object_lock_legal_hold_status {
        cmd.arg("--object-lock-legal-hold-status").arg(value);
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(audit_export_aws_put_object_failed(
            locale,
            output.status.code().unwrap_or(-1),
            &String::from_utf8_lossy(&output.stderr),
        )
        .into());
    }
    Ok(())
}

#[cfg(feature = "gateway")]
async fn upload_to_gcs_via_gsutil(
    locale: Locale,
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
        return Err(audit_export_gsutil_cp_failed(
            locale,
            output.status.code().unwrap_or(-1),
            &String::from_utf8_lossy(&output.stderr),
        )
        .into());
    }
    Ok(())
}

#[cfg(feature = "gateway")]
async fn upload_to_http_put(
    locale: Locale,
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
        return Err(audit_export_http_upload_failed(locale, &status.to_string(), &body).into());
    }
    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!(
        "{}",
        cli_feature_disabled(
            MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US),
            "audit export",
            "--features gateway"
        )
    );
    std::process::exit(2);
}

#[cfg(not(feature = "gateway"))]
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

#[cfg(feature = "gateway")]
fn cli_missing_value(locale: Locale, flag: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.missing_value",
        &[TemplateArg::new("flag", flag)],
    )
}

#[cfg(feature = "gateway")]
fn cli_invalid_value(locale: Locale, label: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.invalid_value",
        &[TemplateArg::new("label", label)],
    )
}

#[cfg(feature = "gateway")]
fn cli_unknown_arg(locale: Locale, arg: &str, usage: Option<&str>) -> String {
    let message =
        MESSAGE_CATALOG.render(locale, "cli.unknown_arg", &[TemplateArg::new("arg", arg)]);
    match usage {
        Some(usage) if !usage.trim().is_empty() => format!("{message}\n{usage}"),
        _ => message,
    }
}

#[cfg(feature = "gateway")]
fn cli_wrote_bytes_to_stdout(locale: Locale, bytes_written: u64) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.wrote_bytes_to_stdout",
        &[TemplateArg::new("bytes_written", bytes_written.to_string())],
    )
}

#[cfg(feature = "gateway")]
fn cli_ok(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "cli.ok", &[])
}

#[cfg(feature = "gateway")]
fn cli_output_path(locale: Locale, path: &str) -> String {
    MESSAGE_CATALOG.render(locale, "cli.output_path", &[TemplateArg::new("path", path)])
}

#[cfg(feature = "gateway")]
fn cli_manifest_path(locale: Locale, path: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.manifest_path",
        &[TemplateArg::new("path", path)],
    )
}

#[cfg(feature = "gateway")]
fn audit_hash_chain_mismatch(locale: Locale, line_no: usize, expected: &str, got: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit.hash_chain_mismatch",
        &[
            TemplateArg::new("line_no", line_no.to_string()),
            TemplateArg::new("expected", expected),
            TemplateArg::new("got", got),
        ],
    )
}

#[cfg(feature = "gateway")]
fn audit_hash_mismatch(locale: Locale, line_no: usize, expected: &str, got: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit.hash_mismatch",
        &[
            TemplateArg::new("line_no", line_no.to_string()),
            TemplateArg::new("expected", expected),
            TemplateArg::new("got", got),
        ],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_usage(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "audit_export.usage", &[])
}

#[cfg(feature = "gateway")]
fn audit_export_cannot_combine_output_upload(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "audit_export.cannot_combine_output_upload", &[])
}

#[cfg(feature = "gateway")]
fn audit_export_missing_admin_token(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "audit_export.missing_admin_token", &[])
}

#[cfg(feature = "gateway")]
fn audit_export_failed_to_read_admin_token(locale: Locale, error: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.failed_to_read_admin_token",
        &[TemplateArg::new("error", error)],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_unsupported_format(locale: Locale, format: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.unsupported_format",
        &[TemplateArg::new("format", format)],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_export_failed(locale: Locale, status: &str, body: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.export_failed",
        &[
            TemplateArg::new("status", status),
            TemplateArg::new("body", body),
        ],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_unsupported_upload_destination(locale: Locale, dest: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.unsupported_upload_destination",
        &[TemplateArg::new("dest", dest)],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_aws_put_object_failed(locale: Locale, exit_code: i32, stderr: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.aws_put_object_failed",
        &[
            TemplateArg::new("exit_code", exit_code.to_string()),
            TemplateArg::new("stderr", stderr),
        ],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_gsutil_cp_failed(locale: Locale, exit_code: i32, stderr: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.gsutil_cp_failed",
        &[
            TemplateArg::new("exit_code", exit_code.to_string()),
            TemplateArg::new("stderr", stderr),
        ],
    )
}

#[cfg(feature = "gateway")]
fn audit_export_http_upload_failed(locale: Locale, status: &str, body: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit_export.http_upload_failed",
        &[
            TemplateArg::new("status", status),
            TemplateArg::new("body", body),
        ],
    )
}
