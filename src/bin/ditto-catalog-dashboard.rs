use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use ditto_llm::{
    CapabilityImplementationStatus, CapabilityKind, ReferenceCatalogExpectation,
    ReferenceProviderModelCatalog, builtin_registry, core_provider_reference_catalog_expectations,
};

const DEFAULT_OUT: &str = "CATALOG_COMPLETENESS.md";
const MAX_MODEL_LIST: usize = 12;

fn main() -> Result<(), String> {
    let mut out_path = PathBuf::from(DEFAULT_OUT);
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

    let generated = render_dashboard()?;
    let existing = fs::read_to_string(&out_path).unwrap_or_default();

    if check {
        if normalize_newlines(&existing) != normalize_newlines(&generated) {
            return Err(format!(
                "{} is out of date (run `cargo run --all-features --bin ditto-catalog-dashboard`)",
                out_path.display()
            ));
        }
        return Ok(());
    }

    if normalize_newlines(&existing) != normalize_newlines(&generated) {
        fs::write(&out_path, generated)
            .map_err(|err| format!("write {} failed: {err}", out_path.display()))?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct ProviderSummary {
    provider_id: String,
    runtime_present: bool,
    reference_present: bool,
    runtime_model_count: usize,
    reference_model_count: usize,
    matched_model_count: usize,
    missing_reference_models: Vec<String>,
    runtime_only_models: Vec<String>,
    implemented_capabilities: Vec<String>,
    planned_capabilities: Vec<String>,
    blocked_capabilities: Vec<String>,
    missing_capabilities: Vec<String>,
    runtime_only_capabilities: Vec<String>,
    reference_validation_issues: usize,
    expectation_issues: Option<usize>,
    runtime_display_name: Option<String>,
    reference_display_name: Option<String>,
}

fn render_dashboard() -> Result<String, String> {
    let registry = builtin_registry();
    let reference_catalogs = load_reference_catalogs()?;
    let expectations = expectation_map();

    let mut provider_ids = BTreeSet::<String>::new();
    provider_ids.extend(reference_catalogs.keys().cloned());
    provider_ids.extend(
        registry
            .plugins()
            .iter()
            .map(|plugin| plugin.id.to_string()),
    );

    let summaries: Vec<_> = provider_ids
        .iter()
        .map(|provider_id| {
            summarize_provider(
                provider_id,
                registry.plugin(provider_id),
                reference_catalogs.get(provider_id),
                expectations.get(provider_id).copied(),
            )
        })
        .collect();

    let mut out = String::new();
    writeln!(&mut out, "# Catalog Completeness Dashboard").unwrap();
    writeln!(&mut out).unwrap();
    writeln!(
        &mut out,
        "Generated from the compiled builtin runtime registry plus `catalog/provider_models/*`."
    )
    .unwrap();
    writeln!(
        &mut out,
        "For a full repo snapshot, regenerate with `cargo run --all-features --bin ditto-catalog-dashboard`."
    )
    .unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "## Provider Summary").unwrap();
    writeln!(&mut out).unwrap();
    writeln!(
        &mut out,
        "| Provider | Runtime | Reference | Models (match/ref/runtime) | Capabilities (done/planned/blocked/missing) | Validation |"
    )
    .unwrap();
    writeln!(&mut out, "| --- | --- | --- | --- | --- | --- |").unwrap();
    for summary in &summaries {
        let validation = match summary.expectation_issues {
            Some(expectation_issues) => {
                format!(
                    "ref:{} / exp:{}",
                    summary.reference_validation_issues, expectation_issues
                )
            }
            None => format!("ref:{} / exp:n/a", summary.reference_validation_issues),
        };
        writeln!(
            &mut out,
            "| `{}` | {} | {} | {}/{}/{} | {}/{}/{}/{} | {} |",
            summary.provider_id,
            bool_cell(summary.runtime_present),
            bool_cell(summary.reference_present),
            summary.matched_model_count,
            summary.reference_model_count,
            summary.runtime_model_count,
            summary.implemented_capabilities.len(),
            summary.planned_capabilities.len(),
            summary.blocked_capabilities.len(),
            summary.missing_capabilities.len(),
            validation,
        )
        .unwrap();
    }

    writeln!(&mut out).unwrap();
    writeln!(&mut out, "## Provider Details").unwrap();
    for summary in &summaries {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "### `{}`", summary.provider_id).unwrap();
        writeln!(&mut out).unwrap();
        writeln!(
            &mut out,
            "- runtime plugin: {}{}",
            bool_word(summary.runtime_present),
            summary
                .runtime_display_name
                .as_deref()
                .map(|display| format!(" ({display})"))
                .unwrap_or_default()
        )
        .unwrap();
        writeln!(
            &mut out,
            "- reference catalog: {}{}",
            bool_word(summary.reference_present),
            summary
                .reference_display_name
                .as_deref()
                .map(|display| format!(" ({display})"))
                .unwrap_or_default()
        )
        .unwrap();
        writeln!(
            &mut out,
            "- models: matched {} / reference {} / runtime {}",
            summary.matched_model_count, summary.reference_model_count, summary.runtime_model_count
        )
        .unwrap();
        writeln!(
            &mut out,
            "- capability coverage (reference scope): done {} / planned {} / blocked {} / missing {}",
            summary.implemented_capabilities.len(),
            summary.planned_capabilities.len(),
            summary.blocked_capabilities.len(),
            summary.missing_capabilities.len()
        )
        .unwrap();
        writeln!(
            &mut out,
            "- reference validation issues: {}",
            summary.reference_validation_issues
        )
        .unwrap();
        match summary.expectation_issues {
            Some(count) => writeln!(&mut out, "- expectation issues: {count}").unwrap(),
            None => writeln!(&mut out, "- expectation issues: n/a").unwrap(),
        }

        writeln!(&mut out).unwrap();
        writeln!(&mut out, "| Capability bucket | Entries |").unwrap();
        writeln!(&mut out, "| --- | --- |").unwrap();
        writeln!(
            &mut out,
            "| Implemented | {} |",
            format_items(&summary.implemented_capabilities)
        )
        .unwrap();
        writeln!(
            &mut out,
            "| Planned | {} |",
            format_items(&summary.planned_capabilities)
        )
        .unwrap();
        writeln!(
            &mut out,
            "| Blocked | {} |",
            format_items(&summary.blocked_capabilities)
        )
        .unwrap();
        writeln!(
            &mut out,
            "| Missing runtime coverage | {} |",
            format_items(&summary.missing_capabilities)
        )
        .unwrap();
        writeln!(
            &mut out,
            "| Runtime-only capability entries | {} |",
            format_items(&summary.runtime_only_capabilities)
        )
        .unwrap();

        writeln!(&mut out).unwrap();
        writeln!(
            &mut out,
            "- missing reference models: {}",
            format_items_truncated(&summary.missing_reference_models, MAX_MODEL_LIST)
        )
        .unwrap();
        writeln!(
            &mut out,
            "- runtime-only models: {}",
            format_items_truncated(&summary.runtime_only_models, MAX_MODEL_LIST)
        )
        .unwrap();
    }

    Ok(out)
}

fn summarize_provider(
    provider_id: &str,
    runtime_plugin: Option<&ditto_llm::ProviderPluginDescriptor>,
    reference_catalog: Option<&ReferenceProviderModelCatalog>,
    expectation: Option<ReferenceCatalogExpectation>,
) -> ProviderSummary {
    let runtime_models = runtime_plugin.map(|plugin| plugin.models()).unwrap_or(&[]);
    let reference_model_ids = reference_catalog
        .map(|catalog| catalog.models.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    let mut matched_model_count = 0usize;
    let mut missing_reference_models = Vec::<String>::new();
    for model_id in &reference_model_ids {
        if runtime_models
            .iter()
            .any(|runtime_model| runtime_model.matches(model_id))
        {
            matched_model_count += 1;
        } else {
            missing_reference_models.push(model_id.clone());
        }
    }

    let runtime_only_models = runtime_models
        .iter()
        .filter(|runtime_model| {
            !reference_model_ids
                .iter()
                .any(|model_id| runtime_model.matches(model_id))
        })
        .map(|model| model.id.to_string())
        .collect::<Vec<_>>();

    let reference_profile = reference_catalog.map(|catalog| catalog.capability_profile());
    let mut runtime_statuses = BTreeMap::<CapabilityKind, CapabilityImplementationStatus>::new();
    if let Some(plugin) = runtime_plugin {
        for descriptor in plugin.capability_statuses() {
            runtime_statuses.insert(descriptor.capability, descriptor.status);
        }
    }

    let mut capability_union = BTreeSet::<CapabilityKind>::new();
    if let Some(profile) = &reference_profile {
        capability_union.extend(profile.capabilities.iter());
    }
    capability_union.extend(runtime_statuses.keys().copied());

    let mut implemented_capabilities = Vec::<String>::new();
    let mut planned_capabilities = Vec::<String>::new();
    let mut blocked_capabilities = Vec::<String>::new();
    let mut missing_capabilities = Vec::<String>::new();
    let mut runtime_only_capabilities = Vec::<String>::new();

    for capability in capability_union {
        let capability_name = capability.to_string();
        let reference_has = reference_profile
            .as_ref()
            .is_some_and(|profile| profile.capabilities.contains(capability));
        match (reference_has, runtime_statuses.get(&capability).copied()) {
            (true, Some(CapabilityImplementationStatus::Implemented)) => {
                implemented_capabilities.push(capability_name)
            }
            (true, Some(CapabilityImplementationStatus::Planned)) => {
                planned_capabilities.push(capability_name)
            }
            (true, Some(CapabilityImplementationStatus::Blocked)) => {
                blocked_capabilities.push(capability_name)
            }
            (true, None) => missing_capabilities.push(capability_name),
            (false, Some(status)) => runtime_only_capabilities.push(format!(
                "{} ({})",
                capability_name,
                status_label(status)
            )),
            (false, None) => {}
        }
    }

    let reference_validation_issues = reference_catalog
        .map(|catalog| catalog.validate(Some(provider_id)).issues.len())
        .unwrap_or(0);
    let expectation_issues = expectation.map(|expectation| {
        reference_catalog
            .map(|catalog| catalog.validate_expectation(expectation).issues.len())
            .unwrap_or(1)
    });

    ProviderSummary {
        provider_id: provider_id.to_string(),
        runtime_present: runtime_plugin.is_some(),
        reference_present: reference_catalog.is_some(),
        runtime_model_count: runtime_models.len(),
        reference_model_count: reference_model_ids.len(),
        matched_model_count,
        missing_reference_models,
        runtime_only_models,
        implemented_capabilities,
        planned_capabilities,
        blocked_capabilities,
        missing_capabilities,
        runtime_only_capabilities,
        reference_validation_issues,
        expectation_issues,
        runtime_display_name: runtime_plugin.map(|plugin| plugin.display_name.to_string()),
        reference_display_name: reference_catalog
            .and_then(|catalog| catalog.provider.display_name.clone()),
    }
}

fn load_reference_catalogs() -> Result<BTreeMap<String, ReferenceProviderModelCatalog>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("catalog")
        .join("provider_models");
    let mut stems = BTreeSet::<String>::new();
    for entry in
        fs::read_dir(&dir).map_err(|err| format!("read {} failed: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| format!("read dir entry failed: {err}"))?;
        let path = entry.path();
        let ext = path.extension().and_then(|value| value.to_str());
        if !matches!(ext, Some("json") | Some("toml")) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            stems.insert(stem.to_string());
        }
    }

    let mut out = BTreeMap::new();
    for stem in stems {
        let json_path = dir.join(format!("{stem}.json"));
        let toml_path = dir.join(format!("{stem}.toml"));
        let path = if json_path.exists() {
            json_path
        } else {
            toml_path
        };
        let catalog = ReferenceProviderModelCatalog::from_path(&path)
            .map_err(|err| format!("load {} failed: {err}", path.display()))?;
        out.insert(catalog.provider.id.clone(), catalog);
    }
    Ok(out)
}

fn expectation_map() -> BTreeMap<String, ReferenceCatalogExpectation> {
    core_provider_reference_catalog_expectations()
        .iter()
        .map(|expectation| (expectation.provider_id.to_string(), *expectation))
        .collect()
}

fn bool_cell(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn bool_word(value: bool) -> &'static str {
    if value { "present" } else { "missing" }
}

fn status_label(status: CapabilityImplementationStatus) -> &'static str {
    match status {
        CapabilityImplementationStatus::Implemented => "implemented",
        CapabilityImplementationStatus::Planned => "planned",
        CapabilityImplementationStatus::Blocked => "blocked",
    }
}

fn format_items(items: &[String]) -> String {
    if items.is_empty() {
        return "-".to_string();
    }
    items.join(", ")
}

fn format_items_truncated(items: &[String], max_items: usize) -> String {
    if items.is_empty() {
        return "-".to_string();
    }
    let mut out = items.iter().take(max_items).cloned().collect::<Vec<_>>();
    if items.len() > max_items {
        out.push(format!("... +{} more", items.len() - max_items));
    }
    out.join(", ")
}

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n")
}
