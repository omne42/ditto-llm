use ditto_core::resources::{MESSAGE_CATALOG, bootstrap_cli_runtime_from_args_with_defaults};
use i18n_kit::{Locale, TemplateArg};
#[cfg(feature = "gateway")]
use omne_integrity_primitives::hash_sha256_json_chain;

#[cfg(feature = "gateway")]
fn main() {
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
        &[TemplateArg::new(
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
                        &[TemplateArg::new("flag", "--input")],
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
                    &[TemplateArg::new("arg", other)],
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
        let expected_hash = hash_sha256_json_chain(prev_hash.as_deref(), &base)?.to_string();
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
            TemplateArg::new("line_no", line_no.to_string()),
            TemplateArg::new("expected", expected),
            TemplateArg::new("got", got),
        ],
    )
}

#[cfg(feature = "gateway")]
fn hash_mismatch(locale: Locale, line_no: usize, expected: &str, got: &str) -> String {
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

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!(
        "{}",
        MESSAGE_CATALOG.render(
            MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US),
            "cli.feature_disabled",
            &[
                TemplateArg::new("feature", "audit verify"),
                TemplateArg::new("rebuild_hint", "--features gateway"),
            ],
        )
    );
    std::process::exit(2);
}
