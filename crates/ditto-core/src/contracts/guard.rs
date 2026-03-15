//! Gateway contract guard — validates OpenAPI contract changes with semver gating.
//!
//! Requires the `contract-guard` feature.

#![cfg(feature = "contract-guard")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_yaml::Value;

use crate::MESSAGE_CATALOG;
use crate::i18n::{Locale, MessageArg, MessageCatalogExt as _};

const DEFAULT_HEAD_OPENAPI: &str = "contracts/gateway-contract-v0.1.openapi.yaml";
const DEFAULT_CONTRACT_LIB: &str = "crates/ditto-server/src/gateway/contracts/types.rs";
const DEFAULT_CONTRACT_CARGO: &str = "crates/ditto-server/Cargo.toml";

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "patch", "options", "head", "trace",
];

#[derive(Debug, Clone)]
pub(crate) struct Args {
    pub base_openapi: Option<PathBuf>,
    pub head_openapi: PathBuf,
    pub contract_lib: PathBuf,
    pub contract_cargo: PathBuf,
    pub allow_missing_base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Semver {
    major: u64,
    minor: u64,
    patch: u64,
}

impl std::fmt::Display for Semver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Default)]
struct ContractShape {
    operations: BTreeMap<String, OperationShape>,
    schemas: BTreeMap<String, SchemaShape>,
}

#[derive(Debug, Default)]
struct OperationShape {
    required_params: BTreeSet<String>,
    request_body_required: bool,
    response_codes: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct SchemaShape {
    required_fields: BTreeSet<String>,
    property_types: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    NoChange,
    Patch,
    Feature,
    Breaking,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoChange => write!(f, "none"),
            Self::Patch => write!(f, "patch"),
            Self::Feature => write!(f, "feature"),
            Self::Breaking => write!(f, "breaking"),
        }
    }
}

#[derive(Debug, Default)]
struct DiffResult {
    breaking: Vec<String>,
    non_breaking: Vec<String>,
}

pub fn cli_main() {
    let raw_args = std::env::args().skip(1).collect::<Vec<_>>();
    let (locale, args) = match MESSAGE_CATALOG.resolve_cli_locale(raw_args, "DITTO_LOCALE") {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    match parse_args(args.into_iter(), locale).and_then(|args| run(args, locale)) {
        Ok(()) => {}
        Err(err) => {
            eprintln!(
                "{}",
                MESSAGE_CATALOG.render(
                    locale,
                    "cli.contract_guard_failed",
                    &[MessageArg::new("error", err)],
                )
            );
            std::process::exit(1);
        }
    }
}

pub(crate) fn parse_args(
    mut args: impl Iterator<Item = String>,
    locale: Locale,
) -> Result<Args, String> {
    let mut parsed = Args {
        base_openapi: None,
        head_openapi: PathBuf::from(DEFAULT_HEAD_OPENAPI),
        contract_lib: PathBuf::from(DEFAULT_CONTRACT_LIB),
        contract_cargo: PathBuf::from(DEFAULT_CONTRACT_CARGO),
        allow_missing_base: false,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => {
                parsed.base_openapi = Some(
                    args.next()
                        .ok_or_else(|| {
                            MESSAGE_CATALOG.render(
                                locale,
                                "cli.missing_value",
                                &[MessageArg::new("flag", "--base")],
                            )
                        })?
                        .into(),
                );
            }
            "--head" => {
                parsed.head_openapi = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[MessageArg::new("flag", "--head")],
                        )
                    })?
                    .into();
            }
            "--contract-lib" => {
                parsed.contract_lib = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[MessageArg::new("flag", "--contract-lib")],
                        )
                    })?
                    .into();
            }
            "--contract-cargo" => {
                parsed.contract_cargo = args
                    .next()
                    .ok_or_else(|| {
                        MESSAGE_CATALOG.render(
                            locale,
                            "cli.missing_value",
                            &[MessageArg::new("flag", "--contract-cargo")],
                        )
                    })?
                    .into();
            }
            "--allow-missing-base" => {
                parsed.allow_missing_base = true;
            }
            "--help" | "-h" => {
                return Err(usage(locale));
            }
            other => {
                return Err(format!(
                    "{}\n{}",
                    MESSAGE_CATALOG.render(
                        locale,
                        "cli.unknown_arg",
                        &[MessageArg::new("arg", other)],
                    ),
                    usage(locale)
                ));
            }
        }
    }

    Ok(parsed)
}

fn usage(locale: Locale) -> String {
    MESSAGE_CATALOG.render(
        locale,
        "cli.usage",
        &[MessageArg::new(
            "command_and_syntax",
            "ditto-gateway-contract-guard [--lang LOCALE] [--base PATH] [--head PATH] [--contract-lib PATH] [--contract-cargo PATH] [--allow-missing-base]",
        )],
    )
}

pub(crate) fn run(args: Args, locale: Locale) -> Result<(), String> {
    let head_raw = std::fs::read_to_string(&args.head_openapi)
        .map_err(|err| format!("read head openapi `{}`: {err}", args.head_openapi.display()))?;
    let head_doc = parse_yaml(&head_raw, &args.head_openapi)?;
    let head_version_text = extract_openapi_version(&head_doc)?;
    let head_version = parse_semver(&head_version_text)?;
    let head_contract_id = extract_openapi_contract_id(&head_doc)?;

    let lib_version = extract_contract_lib_const(&args.contract_lib, "GATEWAY_CONTRACT_VERSION")?;
    let lib_contract_id = extract_contract_lib_const(&args.contract_lib, "GATEWAY_CONTRACT_ID")?;
    let cargo_version = extract_contract_cargo_version(&args.contract_cargo)?;

    if lib_version != head_version_text {
        return Err(format!(
            "version mismatch: openapi info.version={} but {} has GATEWAY_CONTRACT_VERSION={}",
            head_version_text,
            args.contract_lib.display(),
            lib_version
        ));
    }
    if cargo_version != head_version_text {
        return Err(format!(
            "version mismatch: openapi info.version={} but {} package.version={}",
            head_version_text,
            args.contract_cargo.display(),
            cargo_version
        ));
    }
    if lib_contract_id != head_contract_id {
        return Err(format!(
            "contract id mismatch: openapi x-ditto-contract-id={} but {} has GATEWAY_CONTRACT_ID={}",
            head_contract_id,
            args.contract_lib.display(),
            lib_contract_id
        ));
    }

    let base_path = args.base_openapi.as_ref();
    if base_path.is_none() {
        if args.allow_missing_base {
            println!(
                "{}",
                MESSAGE_CATALOG.render(locale, "cli.contract_guard_consistency_only_no_base", &[])
            );
            return Ok(());
        }
        return Err(MESSAGE_CATALOG.render(
            locale,
            "cli.requires",
            &[
                MessageArg::new("flag", "--base"),
                MessageArg::new("requirement", "--allow-missing-base for bootstrap mode"),
            ],
        ));
    }

    let base_path = base_path.expect("checked is_some");
    if !base_path.exists() {
        if args.allow_missing_base {
            println!(
                "{}",
                MESSAGE_CATALOG.render(
                    locale,
                    "cli.contract_guard_consistency_only_missing_base",
                    &[MessageArg::new("path", base_path.display().to_string())],
                )
            );
            return Ok(());
        }
        return Err(MESSAGE_CATALOG.render(
            locale,
            "cli.contract_guard_base_not_found",
            &[MessageArg::new("path", base_path.display().to_string())],
        ));
    }

    let base_raw = std::fs::read_to_string(base_path)
        .map_err(|err| format!("read base openapi `{}`: {err}", base_path.display()))?;
    let base_doc = parse_yaml(&base_raw, base_path)?;
    let base_version_text = extract_openapi_version(&base_doc)?;
    let base_version = parse_semver(&base_version_text)?;

    let base_shape = collect_shape(&base_doc)?;
    let head_shape = collect_shape(&head_doc)?;
    let diff = diff_shape(&base_shape, &head_shape);

    let change_kind = if !diff.breaking.is_empty() {
        ChangeKind::Breaking
    } else if !diff.non_breaking.is_empty() {
        ChangeKind::Feature
    } else if base_raw != head_raw {
        ChangeKind::Patch
    } else {
        ChangeKind::NoChange
    };

    enforce_semver_gate(base_version, head_version, change_kind)?;

    println!(
        "{}",
        MESSAGE_CATALOG.render(
            locale,
            "cli.contract_guard_summary",
            &[
                MessageArg::new("base_version", base_version.to_string()),
                MessageArg::new("head_version", head_version.to_string()),
                MessageArg::new("change_kind", change_kind.to_string()),
            ],
        )
    );
    for reason in &diff.breaking {
        println!(
            "{}",
            MESSAGE_CATALOG.render(
                locale,
                "cli.breaking",
                &[MessageArg::new("reason", reason.as_str())],
            )
        );
    }
    for reason in &diff.non_breaking {
        println!(
            "{}",
            MESSAGE_CATALOG.render(
                locale,
                "cli.non_breaking",
                &[MessageArg::new("reason", reason.as_str())],
            )
        );
    }
    if change_kind == ChangeKind::Patch {
        println!(
            "{}",
            MESSAGE_CATALOG.render(locale, "cli.textual_only_contract_edits", &[])
        );
    }

    Ok(())
}

fn parse_yaml(raw: &str, path: &Path) -> Result<Value, String> {
    serde_yaml::from_str(raw).map_err(|err| format!("parse yaml `{}`: {err}", path.display()))
}

fn yaml_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_mapping()?.get(Value::String(key.to_string()))
}

fn value_as_string(value: &Value) -> Option<String> {
    value.as_str().map(|s| s.to_string())
}

fn extract_openapi_version(doc: &Value) -> Result<String, String> {
    yaml_get(doc, "info")
        .and_then(|info| yaml_get(info, "version"))
        .and_then(value_as_string)
        .ok_or("missing info.version in openapi".to_string())
}

fn extract_openapi_contract_id(doc: &Value) -> Result<String, String> {
    yaml_get(doc, "info")
        .and_then(|info| yaml_get(info, "x-ditto-contract-id"))
        .and_then(value_as_string)
        .ok_or("missing info.x-ditto-contract-id in openapi".to_string())
}

fn extract_contract_lib_const(path: &Path, const_name: &str) -> Result<String, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| format!("read `{}`: {err}", path.display()))?;
    let pattern = format!(
        r#"(?m)^\s*pub\s+const\s+{}\s*:\s*&str\s*=\s*"([^"]+)""#,
        regex::escape(const_name)
    );
    let re = Regex::new(&pattern).map_err(|err| format!("compile regex: {err}"))?;
    let captures = re
        .captures(&raw)
        .ok_or_else(|| format!("constant `{const_name}` not found in `{}`", path.display()))?;
    Ok(captures
        .get(1)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("constant `{const_name}` has no captured value"))?)
}

fn extract_contract_cargo_version(path: &Path) -> Result<String, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| format!("read `{}`: {err}", path.display()))?;
    let doc: toml::Value =
        toml::from_str(&raw).map_err(|err| format!("parse `{}` as toml: {err}", path.display()))?;
    doc.get("package")
        .and_then(|pkg| pkg.get("version"))
        .and_then(toml::Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| format!("missing package.version in `{}`", path.display()))
}

fn parse_semver(raw: &str) -> Result<Semver, String> {
    let mut parts = raw.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("invalid semver `{raw}`"))?
        .parse::<u64>()
        .map_err(|_| format!("invalid semver `{raw}`"))?;
    let minor = parts
        .next()
        .ok_or_else(|| format!("invalid semver `{raw}`"))?
        .parse::<u64>()
        .map_err(|_| format!("invalid semver `{raw}`"))?;
    let patch = parts
        .next()
        .ok_or_else(|| format!("invalid semver `{raw}`"))?
        .parse::<u64>()
        .map_err(|_| format!("invalid semver `{raw}`"))?;
    if parts.next().is_some() {
        return Err(format!("invalid semver `{raw}`"));
    }
    Ok(Semver {
        major,
        minor,
        patch,
    })
}

fn collect_shape(doc: &Value) -> Result<ContractShape, String> {
    let mut out = ContractShape::default();

    let paths = yaml_get(doc, "paths")
        .and_then(Value::as_mapping)
        .ok_or("missing paths in openapi".to_string())?;
    for (path_key, path_value) in paths {
        let Some(path) = path_key.as_str() else {
            continue;
        };
        let Some(path_item) = path_value.as_mapping() else {
            continue;
        };
        let path_level_required = collect_required_params(yaml_get(path_value, "parameters"));
        for method in METHODS {
            let Some(op) = path_item.get(Value::String(method.to_string())) else {
                continue;
            };
            let mut required_params = path_level_required.clone();
            required_params.extend(collect_required_params(yaml_get(op, "parameters")));
            let request_body_required = yaml_get(op, "requestBody")
                .and_then(|body| yaml_get(body, "required"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let response_codes = collect_response_codes(yaml_get(op, "responses"));

            out.operations.insert(
                format!("{} {}", method.to_ascii_uppercase(), path),
                OperationShape {
                    required_params,
                    request_body_required,
                    response_codes,
                },
            );
        }
    }

    if let Some(schemas) = yaml_get(doc, "components")
        .and_then(|components| yaml_get(components, "schemas"))
        .and_then(Value::as_mapping)
    {
        for (name_key, schema_value) in schemas {
            let Some(name) = name_key.as_str() else {
                continue;
            };
            let required_fields = collect_required_fields(yaml_get(schema_value, "required"));
            let property_types = collect_property_types(yaml_get(schema_value, "properties"));
            out.schemas.insert(
                name.to_string(),
                SchemaShape {
                    required_fields,
                    property_types,
                },
            );
        }
    }

    Ok(out)
}

fn collect_required_params(params: Option<&Value>) -> BTreeSet<String> {
    let mut required = BTreeSet::new();
    let Some(params) = params.and_then(Value::as_sequence) else {
        return required;
    };
    for param in params {
        let is_required = yaml_get(param, "required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !is_required {
            continue;
        }
        let Some(location) = yaml_get(param, "in").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = yaml_get(param, "name").and_then(Value::as_str) else {
            continue;
        };
        required.insert(format!("{location}:{name}"));
    }
    required
}

fn collect_response_codes(responses: Option<&Value>) -> BTreeSet<String> {
    let mut codes = BTreeSet::new();
    let Some(map) = responses.and_then(Value::as_mapping) else {
        return codes;
    };
    for key in map.keys() {
        if let Some(code) = key.as_str() {
            codes.insert(code.to_string());
        }
    }
    codes
}

fn collect_required_fields(required: Option<&Value>) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();
    let Some(list) = required.and_then(Value::as_sequence) else {
        return fields;
    };
    for item in list {
        if let Some(field) = item.as_str() {
            fields.insert(field.to_string());
        }
    }
    fields
}

fn collect_property_types(properties: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(map) = properties.and_then(Value::as_mapping) else {
        return out;
    };
    for (key, value) in map {
        let Some(name) = key.as_str() else {
            continue;
        };
        let shape = if let Some(typ) = yaml_get(value, "type").and_then(Value::as_str) {
            if let Some(format) = yaml_get(value, "format").and_then(Value::as_str) {
                format!("{typ}:{format}")
            } else {
                typ.to_string()
            }
        } else if let Some(reference) = yaml_get(value, "$ref").and_then(Value::as_str) {
            format!("$ref:{reference}")
        } else if yaml_get(value, "oneOf").is_some() {
            "oneOf".to_string()
        } else if yaml_get(value, "allOf").is_some() {
            "allOf".to_string()
        } else if yaml_get(value, "anyOf").is_some() {
            "anyOf".to_string()
        } else {
            "unknown".to_string()
        };
        out.insert(name.to_string(), shape);
    }
    out
}

fn diff_shape(base: &ContractShape, head: &ContractShape) -> DiffResult {
    let mut breaking = BTreeSet::new();
    let mut non_breaking = BTreeSet::new();

    for (op, base_op) in &base.operations {
        match head.operations.get(op) {
            None => {
                breaking.insert(format!("removed operation `{op}`"));
            }
            Some(head_op) => {
                for required in head_op.required_params.difference(&base_op.required_params) {
                    breaking.insert(format!(
                        "operation `{op}` added required param `{required}`"
                    ));
                }
                for required in base_op.required_params.difference(&head_op.required_params) {
                    non_breaking.insert(format!(
                        "operation `{op}` removed required param `{required}`"
                    ));
                }
                if !base_op.request_body_required && head_op.request_body_required {
                    breaking.insert(format!("operation `{op}` requestBody became required"));
                }
                if base_op.request_body_required && !head_op.request_body_required {
                    non_breaking.insert(format!("operation `{op}` requestBody became optional"));
                }
                for code in base_op.response_codes.difference(&head_op.response_codes) {
                    breaking.insert(format!("operation `{op}` removed response code `{code}`"));
                }
                for code in head_op.response_codes.difference(&base_op.response_codes) {
                    non_breaking.insert(format!("operation `{op}` added response code `{code}`"));
                }
            }
        }
    }
    for op in head.operations.keys() {
        if !base.operations.contains_key(op) {
            non_breaking.insert(format!("added operation `{op}`"));
        }
    }

    for (schema, base_schema) in &base.schemas {
        match head.schemas.get(schema) {
            None => {
                breaking.insert(format!("removed schema `{schema}`"));
            }
            Some(head_schema) => {
                for field in head_schema
                    .required_fields
                    .difference(&base_schema.required_fields)
                {
                    breaking.insert(format!("schema `{schema}` added required field `{field}`"));
                }
                for field in base_schema
                    .required_fields
                    .difference(&head_schema.required_fields)
                {
                    non_breaking.insert(format!(
                        "schema `{schema}` removed required constraint for field `{field}`"
                    ));
                }
                for (property, base_type) in &base_schema.property_types {
                    match head_schema.property_types.get(property) {
                        None => {
                            breaking
                                .insert(format!("schema `{schema}` removed property `{property}`"));
                        }
                        Some(head_type) if head_type != base_type => {
                            breaking.insert(format!(
                                "schema `{schema}` changed property `{property}` type from `{base_type}` to `{head_type}`"
                            ));
                        }
                        _ => {}
                    }
                }
                for property in head_schema.property_types.keys() {
                    if !base_schema.property_types.contains_key(property) {
                        non_breaking
                            .insert(format!("schema `{schema}` added property `{property}`"));
                    }
                }
            }
        }
    }
    for schema in head.schemas.keys() {
        if !base.schemas.contains_key(schema) {
            non_breaking.insert(format!("added schema `{schema}`"));
        }
    }

    DiffResult {
        breaking: breaking.into_iter().collect(),
        non_breaking: non_breaking.into_iter().collect(),
    }
}

fn enforce_semver_gate(base: Semver, head: Semver, change_kind: ChangeKind) -> Result<(), String> {
    if head < base {
        return Err(format!(
            "openapi info.version decreased: base={} head={}",
            base, head
        ));
    }

    match change_kind {
        ChangeKind::NoChange => Ok(()),
        ChangeKind::Patch => {
            if head == base {
                Err(format!(
                    "contract changed but version not bumped (base={} head={})",
                    base, head
                ))
            } else {
                Ok(())
            }
        }
        ChangeKind::Feature => {
            if is_feature_bump(base, head) {
                Ok(())
            } else {
                Err(format!(
                    "non-breaking contract changes require minor/major bump (base={} head={})",
                    base, head
                ))
            }
        }
        ChangeKind::Breaking => {
            if is_breaking_bump(base, head) {
                Ok(())
            } else {
                Err(format!(
                    "breaking contract changes require major bump (or minor when major=0) (base={} head={})",
                    base, head
                ))
            }
        }
    }
}

fn is_feature_bump(base: Semver, head: Semver) -> bool {
    head.major > base.major || (head.major == base.major && head.minor > base.minor)
}

fn is_breaking_bump(base: Semver, head: Semver) -> bool {
    if head.major > base.major {
        return true;
    }
    base.major == 0 && head.major == 0 && head.minor > base.minor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_doc(raw: &str) -> Value {
        serde_yaml::from_str(raw).expect("parse yaml")
    }

    const BASE: &str = r#"
openapi: 3.0.3
info:
  version: 0.1.0
  x-ditto-contract-id: gateway-v0.1
paths:
  /health:
    get:
      responses:
        '200':
          description: ok
  /v1/chat/completions:
    post:
      parameters:
        - in: query
          name: mode
          required: false
          schema:
            type: string
      requestBody:
        required: true
      responses:
        '200':
          description: ok
components:
  schemas:
    HealthResponse:
      type: object
      required: [status]
      properties:
        status:
          type: string
"#;

    #[test]
    fn detects_removed_operation_as_breaking() {
        let base = collect_shape(&parse_doc(BASE)).expect("shape");
        let head = collect_shape(&parse_doc(
            r#"
openapi: 3.0.3
info:
  version: 0.2.0
  x-ditto-contract-id: gateway-v0.1
paths:
  /health:
    get:
      responses:
        '200':
          description: ok
"#,
        ))
        .expect("shape");

        let diff = diff_shape(&base, &head);
        assert!(!diff.breaking.is_empty());
        assert!(
            diff.breaking
                .iter()
                .any(|line| line.contains("removed operation `POST /v1/chat/completions`"))
        );
    }

    #[test]
    fn detects_added_operation_as_non_breaking() {
        let base = collect_shape(&parse_doc(BASE)).expect("shape");
        let head = collect_shape(&parse_doc(
            r#"
openapi: 3.0.3
info:
  version: 0.2.0
  x-ditto-contract-id: gateway-v0.1
paths:
  /health:
    get:
      responses:
        '200':
          description: ok
  /v1/chat/completions:
    post:
      requestBody:
        required: true
      responses:
        '200':
          description: ok
  /admin/audit:
    get:
      responses:
        '200':
          description: ok
components:
  schemas:
    HealthResponse:
      type: object
      required: [status]
      properties:
        status:
          type: string
"#,
        ))
        .expect("shape");

        let diff = diff_shape(&base, &head);
        assert!(diff.breaking.is_empty());
        assert!(
            diff.non_breaking
                .iter()
                .any(|line| line.contains("added operation `GET /admin/audit`"))
        );
    }

    #[test]
    fn semver_gate_rules() {
        let base = Semver {
            major: 0,
            minor: 1,
            patch: 0,
        };

        let patch = Semver {
            major: 0,
            minor: 1,
            patch: 1,
        };
        assert!(enforce_semver_gate(base, patch, ChangeKind::Patch).is_ok());
        assert!(enforce_semver_gate(base, patch, ChangeKind::Feature).is_err());

        let minor = Semver {
            major: 0,
            minor: 2,
            patch: 0,
        };
        assert!(enforce_semver_gate(base, minor, ChangeKind::Feature).is_ok());
        assert!(enforce_semver_gate(base, minor, ChangeKind::Breaking).is_ok());
    }

    #[test]
    fn semver_parser_rejects_invalid_values() {
        assert!(parse_semver("0.1").is_err());
        assert!(parse_semver("x.y.z").is_err());
        assert!(parse_semver("1.2.3.4").is_err());
        assert_eq!(parse_semver("1.2.3").expect("parse").to_string(), "1.2.3");
    }
}
