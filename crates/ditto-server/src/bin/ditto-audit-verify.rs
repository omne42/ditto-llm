use ditto_core::MESSAGE_CATALOG;
use ditto_core::i18n::{Locale, MessageArg, MessageCatalogExt as _};

#[cfg(feature = "gateway")]
fn main() {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    let (locale, args) = match MESSAGE_CATALOG.resolve_cli_locale(raw_args, "DITTO_LOCALE") {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    if let Err(err) = run(locale, args) {
        eprintln!("{}", render_error(err.as_ref(), locale));
        std::process::exit(1);
    }
}

#[cfg(feature = "gateway")]
fn run(locale: Locale, raw_args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufRead;

    let usage = MESSAGE_CATALOG.render(
        locale,
        "cli.usage",
        &[MessageArg::new(
            "command_and_syntax",
            "ditto-audit-verify --input PATH|-",
        )],
    );
    let mut args = raw_args.into_iter();

    let mut input: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input = Some(args.next().ok_or_else(|| {
                    MESSAGE_CATALOG.render(
                        locale,
                        "cli.missing_value",
                        &[MessageArg::new("flag", "--input")],
                    )
                })?);
            }
            "--help" | "-h" => {
                println!("{usage}");
                return Ok(());
            }
            other => {
                let message = MESSAGE_CATALOG.render(
                    locale,
                    "cli.unknown_arg",
                    &[MessageArg::new("arg", other)],
                );
                return Err(format!("{message}\n{usage}").into());
            }
        }
    }

    let input = input.ok_or_else(|| usage.clone())?;

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

    fn hex_lower(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len().saturating_mul(2));
        for byte in bytes {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }

    fn audit_chain_hash(
        prev_hash: Option<&str>,
        record: &ditto_server::gateway::AuditLogRecord,
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

    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        let file = std::fs::File::open(&input)?;
        Box::new(std::io::BufReader::new(file))
    };

    let mut prev_hash: Option<String> = None;
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
            return Err(hash_chain_mismatch(locale, line_no + 1, expected_prev, got_prev).into());
        }

        let base = ditto_server::gateway::AuditLogRecord {
            id: record.id,
            ts_ms: record.ts_ms,
            kind: record.kind,
            payload: record.payload,
        };
        let expected_hash = audit_chain_hash(prev_hash.as_deref(), &base);
        if record.hash != expected_hash {
            return Err(hash_mismatch(locale, line_no + 1, &expected_hash, &record.hash).into());
        }
        prev_hash = Some(record.hash);
    }

    println!("{}", MESSAGE_CATALOG.render(locale, "cli.ok", &[]));
    Ok(())
}

#[cfg(feature = "gateway")]
fn hash_chain_mismatch(locale: Locale, line_no: usize, expected: &str, got: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit.hash_chain_mismatch",
        &[
            MessageArg::new("line_no", line_no.to_string()),
            MessageArg::new("expected", expected),
            MessageArg::new("got", got),
        ],
    )
}

#[cfg(feature = "gateway")]
fn hash_mismatch(locale: Locale, line_no: usize, expected: &str, got: &str) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "audit.hash_mismatch",
        &[
            MessageArg::new("line_no", line_no.to_string()),
            MessageArg::new("expected", expected),
            MessageArg::new("got", got),
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
        &[MessageArg::new("error", error.to_string())],
    )
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!(
        "{}",
        MESSAGE_CATALOG.render(
            MESSAGE_CATALOG.default_locale(),
            "cli.feature_disabled",
            &[
                MessageArg::new("feature", "audit verify"),
                MessageArg::new("rebuild_hint", "--features gateway"),
            ],
        )
    );
    std::process::exit(2);
}
