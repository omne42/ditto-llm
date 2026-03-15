use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use ditto_core::i18n::{MessageArg, MessageCatalogExt as _};
use ditto_core::MESSAGE_CATALOG;

const BEGIN_MARKER: &str = "----- BEGIN AUTO-GENERATED DOCS (ditto-llms-txt) -----";
const END_MARKER: &str = "----- END AUTO-GENERATED DOCS (ditto-llms-txt) -----";

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
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(locale: ditto_core::i18n::Locale, raw_args: Vec<String>) -> Result<(), String> {
    let mut out_paths = vec![
        PathBuf::from("llms.txt"),
        PathBuf::from("docs/src/llms.txt"),
    ];
    let mut has_custom_out = false;
    let mut check = false;

    let usage = MESSAGE_CATALOG.render(
        locale,
        "cli.usage",
        &[MessageArg::new(
            "command_and_syntax",
            "ditto-llms-txt [--out PATH]... [--check]",
        )],
    );
    let mut args = raw_args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[MessageArg::new("flag", "--out")],
                        )
                    })?;
                let path = PathBuf::from(value);
                if !has_custom_out {
                    out_paths.clear();
                    has_custom_out = true;
                }
                out_paths.push(path);
            }
            "--check" => check = true,
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
                return Err(format!("{message}\n{usage}"));
            }
        }
    }

    let mut paths = Vec::<PathBuf>::new();
    for file in [
        "README.md",
        "PROVIDERS.md",
        "CATALOG_COMPLETENESS.md",
        "COMPARED_TO_LITELLM_AI_SDK.md",
    ] {
        let path = PathBuf::from(file);
        if path.exists() {
            paths.push(path);
        }
    }
    paths.extend(collect_docs_paths_from_summary_with_locale(
        "docs/src/SUMMARY.md",
        locale,
    )?);
    paths = dedup_keep_order(paths);

    let generated = format!(
        "{BEGIN_MARKER}\n{}\n{END_MARKER}\n",
        render_files_with_locale(&paths, locale)?
    );

    let regen_hint = regen_hint(&out_paths);

    for out_path in out_paths {
        let existing = fs::read_to_string(&out_path).unwrap_or_default();
        let prelude = split_prelude(&existing);
        let next = format!("{prelude}{generated}");

        if check {
            if normalize_newlines(&existing) != normalize_newlines(&next) {
                return Err(MESSAGE_CATALOG.render(
                    locale,
                    "cli.out_of_date",
                    &[
                        MessageArg::new("path", out_path.display().to_string()),
                        MessageArg::new("command", regen_hint.as_str()),
                    ],
                ));
            }
            continue;
        }

        if normalize_newlines(&existing) == normalize_newlines(&next) {
            continue;
        }

        fs::write(&out_path, next).map_err(|err| {
            MESSAGE_CATALOG.render(
                locale,
                "cli.write_failed",
                &[
                    MessageArg::new("path", out_path.display().to_string()),
                    MessageArg::new("error", err.to_string()),
                ],
            )
        })?;
    }

    Ok(())
}

fn split_prelude(existing: &str) -> String {
    let Some((before, _)) = existing.split_once(BEGIN_MARKER) else {
        let trimmed = existing.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return String::new();
        }
        return format!("{trimmed}\n\n");
    };
    let trimmed = before.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n\n")
    }
}

fn collect_docs_paths_from_summary_with_locale(
    summary_path: impl AsRef<Path>,
    locale: ditto_core::i18n::Locale,
) -> Result<Vec<PathBuf>, String> {
    let summary_path = summary_path.as_ref();
    let contents = fs::read_to_string(summary_path).map_err(|err| {
        MESSAGE_CATALOG.render(
            locale,
            "cli.read_failed",
            &[
                MessageArg::new("path", summary_path.display().to_string()),
                MessageArg::new("error", err.to_string()),
            ],
        )
    })?;
    let base_dir = summary_path
        .parent()
        .ok_or_else(|| {
            MESSAGE_CATALOG.render(
                locale,
                "llms_txt.invalid_summary_path",
                &[MessageArg::new("path", summary_path.display().to_string())],
            )
        })?;

    let mut seen = BTreeSet::<PathBuf>::new();
    let mut out = Vec::<PathBuf>::new();

    for line in contents.lines() {
        let Some((_label, rest)) = line.split_once("](") else {
            continue;
        };
        let Some((raw_target, _)) = rest.split_once(')') else {
            continue;
        };
        let target = raw_target.trim();
        if target.is_empty() || target.starts_with('#') || target.starts_with("http") {
            continue;
        }

        let target = target.strip_prefix("./").unwrap_or(target);
        if !is_markdown_target(target) {
            continue;
        }

        let path = base_dir.join(target);
        let path = normalize_relative_path(&path);
        if !path.exists() {
            return Err(MESSAGE_CATALOG.render(
                locale,
                "llms_txt.summary_missing_file",
                &[MessageArg::new("path", path.display().to_string())],
            ));
        }
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }

    Ok(out)
}

fn render_files_with_locale(
    paths: &[PathBuf],
    locale: ditto_core::i18n::Locale,
) -> Result<String, String> {
    let mut out = String::new();
    for path in paths {
        let content = fs::read_to_string(path).map_err(|err| {
            MESSAGE_CATALOG.render(
                locale,
                "cli.read_failed",
                &[
                    MessageArg::new("path", path.display().to_string()),
                    MessageArg::new("error", err.to_string()),
                ],
            )
        })?;
        out.push_str(
            "================================================================================\n",
        );
        writeln!(&mut out, "FILE: {}", path.display()).expect("writing to string cannot fail");
        out.push_str(
            "================================================================================\n\n",
        );
        out.push_str(&content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(out)
}

fn dedup_keep_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::<PathBuf>::new();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n")
}

fn is_markdown_target(target: &str) -> bool {
    Path::new(target)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn regen_hint(out_paths: &[PathBuf]) -> String {
    if out_paths
        == [
            PathBuf::from("llms.txt"),
            PathBuf::from("docs/src/llms.txt"),
        ]
    {
        "cargo run -p ditto-core --bin ditto-llms-txt".to_string()
    } else {
        let mut cmd = "cargo run -p ditto-core --bin ditto-llms-txt --".to_string();
        for path in out_paths {
            write!(&mut cmd, " --out {}", path.display()).expect("writing to string cannot fail");
        }
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::is_markdown_target;

    #[test]
    fn recognizes_markdown_extension_case_insensitively() {
        assert!(is_markdown_target("README.md"));
        assert!(is_markdown_target("README.MD"));
        assert!(is_markdown_target("docs/src/Guide.Md"));
        assert!(!is_markdown_target("README.mdx"));
        assert!(!is_markdown_target("README"));
    }
}
