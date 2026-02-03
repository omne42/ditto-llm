use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const BEGIN_MARKER: &str = "----- BEGIN AUTO-GENERATED DOCS (ditto-llms-txt) -----";
const END_MARKER: &str = "----- END AUTO-GENERATED DOCS (ditto-llms-txt) -----";

fn main() -> Result<(), String> {
    let mut out_path = PathBuf::from("llms.txt");
    let mut check = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--out requires a value".to_string())?;
                out_path = PathBuf::from(value);
            }
            "--check" => check = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let existing = fs::read_to_string(&out_path).unwrap_or_default();
    let prelude = split_prelude(&existing);

    let mut paths = Vec::<PathBuf>::new();
    for file in ["README.md", "PROVIDERS.md", "COMPARED_TO_LITELLM_AI_SDK.md"] {
        let path = PathBuf::from(file);
        if path.exists() {
            paths.push(path);
        }
    }
    paths.extend(collect_docs_paths_from_summary("docs/src/SUMMARY.md")?);
    paths = dedup_keep_order(paths);

    let generated = format!("{BEGIN_MARKER}\n{}\n{END_MARKER}\n", render_files(&paths)?);

    let next = format!("{prelude}{generated}");

    if check {
        if normalize_newlines(&existing) != normalize_newlines(&next) {
            return Err(format!(
                "{} is out of date (run `cargo run --bin ditto-llms-txt -- --out {}`)",
                out_path.display(),
                out_path.display()
            ));
        }
        return Ok(());
    }

    if normalize_newlines(&existing) == normalize_newlines(&next) {
        return Ok(());
    }

    fs::write(&out_path, next).map_err(|err| format!("write {} failed: {err}", out_path.display()))
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

fn collect_docs_paths_from_summary(summary_path: impl AsRef<Path>) -> Result<Vec<PathBuf>, String> {
    let summary_path = summary_path.as_ref();
    let contents = fs::read_to_string(summary_path)
        .map_err(|err| format!("read {} failed: {err}", summary_path.display()))?;
    let base_dir = summary_path
        .parent()
        .ok_or_else(|| format!("invalid summary path: {}", summary_path.display()))?;

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
        if !target.ends_with(".md") {
            continue;
        }

        let path = base_dir.join(target);
        let path = normalize_relative_path(&path);
        if !path.exists() {
            return Err(format!(
                "SUMMARY link points to missing file: {}",
                path.display()
            ));
        }
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }

    Ok(out)
}

fn render_files(paths: &[PathBuf]) -> Result<String, String> {
    let mut out = String::new();
    for path in paths {
        let content = fs::read_to_string(path)
            .map_err(|err| format!("read {} failed: {err}", path.display()))?;
        out.push_str(
            "================================================================================\n",
        );
        out.push_str(&format!("FILE: {}\n", path.display()));
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
