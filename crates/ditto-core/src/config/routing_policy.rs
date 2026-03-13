use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contracts::{CapabilityKind, OperationKind};

use super::provider_config::{ProviderConfig, normalize_string_list};

// Multi-provider weighted routing config for "completion"/"thinking" phases.
//
// Core ideas:
// - `profiles`: reusable provider endpoints (same provider can appear multiple times with
//   different `auth`/`base_url`/`default_model`).
// - `default` / `by_role` / `by_scenario` / `overrides`: routing policy hierarchy.
// - each stage has `targets` with optional `weight`; resolver returns ordered candidates where
//   first target is weighted-primary and remaining ones are fallback order.
//
// TOML sketch:
// ```toml
// [profiles.fast]
// provider = "compat-primary"
// base_url = "https://proxy.example/v1"
// default_model = "chat-small"
// weight = 3
//
// [profiles.fast.auth]
// type = "api_key_env"
// keys = ["OPENAI_COMPAT_API_KEY_A"]
//
// [profiles.quality]
// provider = "compat-primary"
// base_url = "https://proxy.example/v1"
// default_model = "chat-large"
// weight = 1
//
// [default.completion]
// targets = [{ profile = "fast" }, { profile = "quality" }]
//
// [default.thinking]
// targets = [{ profile = "quality", model = "o3" }]
// ```

fn default_weight() -> f64 {
    1.0
}

const LLM_ROUTING_OPERATIONS: &[OperationKind] = &[
    OperationKind::CHAT_COMPLETION,
    OperationKind::RESPONSE,
    OperationKind::TEXT_COMPLETION,
];

#[derive(Debug, Clone, Copy)]
struct RoutingPhaseRequirement {
    capability: CapabilityKind,
    preferred_operations: &'static [OperationKind],
}

fn routing_phase_requirement(phase: RoutingPhase) -> RoutingPhaseRequirement {
    match phase {
        RoutingPhase::Completion | RoutingPhase::Thinking => RoutingPhaseRequirement {
            capability: CapabilityKind::LLM,
            preferred_operations: LLM_ROUTING_OPERATIONS,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPhase {
    #[default]
    Completion,
    Thinking,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProviderRoutingConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, RoutingProviderProfile>,
    #[serde(default)]
    pub default: RoutingPolicy,
    #[serde(default)]
    pub by_role: BTreeMap<String, RoutingPolicy>,
    #[serde(default)]
    pub by_scenario: BTreeMap<String, RoutingPolicy>,
    #[serde(default)]
    pub overrides: Vec<RoutingOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingProviderProfile {
    #[serde(default)]
    pub provider: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(flatten)]
    pub config: ProviderConfig,
}

impl Default for RoutingProviderProfile {
    fn default() -> Self {
        Self {
            provider: String::new(),
            weight: default_weight(),
            config: ProviderConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutingPolicy {
    #[serde(default)]
    pub completion: RoutingStagePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<RoutingStagePolicy>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutingStagePolicy {
    #[serde(
        default,
        alias = "providers",
        alias = "candidates",
        alias = "callbacks",
        alias = "fallbacks"
    )]
    pub targets: Vec<RoutingTarget>,
    #[serde(default, alias = "fallback_models")]
    pub model_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutingTarget {
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, alias = "fallback_models")]
    pub model_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutingOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<String>,
    #[serde(default)]
    pub completion: RoutingStagePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<RoutingStagePolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPolicySource {
    Default,
    Role,
    Scenario,
    Override,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRoutingTarget {
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub weight: f64,
    pub provider_config: ProviderConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRoutingPlan {
    pub phase: RoutingPhase,
    pub policy_source: RoutingPolicySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_key: Option<String>,
    pub targets: Vec<ResolvedRoutingTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoutingContext<'a> {
    pub role: Option<&'a str>,
    pub scenario: Option<&'a str>,
    pub phase: RoutingPhase,
    pub seed_hash: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingConfigFormat {
    Json,
    Toml,
}

impl ProviderRoutingConfig {
    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|err| format!("parse routing json: {err}"))
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, String> {
        toml::from_str(raw).map_err(|err| format!("parse routing toml: {err}"))
    }

    pub fn from_str_auto(raw: &str) -> Result<Self, String> {
        match Self::from_json_str(raw) {
            Ok(parsed) => Ok(parsed),
            Err(json_err) => {
                Self::from_toml_str(raw).map_err(|toml_err| format!("{json_err}; {toml_err}"))
            }
        }
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let format = match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Some(RoutingConfigFormat::Json),
            Some("toml") => Some(RoutingConfigFormat::Toml),
            _ => None,
        };
        Self::from_str_with_format(&raw, format)
    }

    pub fn from_str_with_format(
        raw: &str,
        format: Option<RoutingConfigFormat>,
    ) -> Result<Self, String> {
        match format {
            Some(RoutingConfigFormat::Json) => Self::from_json_str(raw),
            Some(RoutingConfigFormat::Toml) => Self::from_toml_str(raw),
            None => Self::from_str_auto(raw),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (profile_id, profile) in &self.profiles {
            let profile_id_trimmed = profile_id.trim();
            if profile_id_trimmed.is_empty() {
                return Err("routing.profiles contains empty key".to_string());
            }
            if profile.provider.trim().is_empty() {
                return Err(format!(
                    "routing.profiles.{profile_id}.provider must not be empty"
                ));
            }
            if !weight_valid(profile.weight) {
                return Err(format!(
                    "routing.profiles.{profile_id}.weight must be finite and > 0"
                ));
            }
        }

        validate_policy("routing.default", &self.default, &self.profiles)?;
        for (role, policy) in &self.by_role {
            let role = role.trim();
            if role.is_empty() {
                return Err("routing.by_role contains empty key".to_string());
            }
            validate_policy(&format!("routing.by_role.{role}"), policy, &self.profiles)?;
        }
        for (scenario, policy) in &self.by_scenario {
            let scenario = scenario.trim();
            if scenario.is_empty() {
                return Err("routing.by_scenario contains empty key".to_string());
            }
            validate_policy(
                &format!("routing.by_scenario.{scenario}"),
                policy,
                &self.profiles,
            )?;
        }
        for (idx, rule) in self.overrides.iter().enumerate() {
            if clean_opt_string(rule.role.as_deref()).is_none()
                && clean_opt_string(rule.scenario.as_deref()).is_none()
            {
                return Err(format!(
                    "routing.overrides[{idx}] must set at least one of role/scenario"
                ));
            }
            let policy = RoutingPolicy {
                completion: rule.completion.clone(),
                thinking: rule.thinking.clone(),
            };
            validate_policy(
                &format!("routing.overrides[{idx}]"),
                &policy,
                &self.profiles,
            )?;
        }

        Ok(())
    }

    pub fn resolve_plan(&self, ctx: RoutingContext<'_>) -> Result<ResolvedRoutingPlan, String> {
        self.validate()?;

        let selected_policy = self.select_policy(ctx.role, ctx.scenario);
        let source = selected_policy.source;
        let stage = selected_policy.stage(ctx.phase).clone();
        let policy_key = selected_policy.key;
        let stage_key = if policy_key.is_empty() {
            source.to_string()
        } else {
            format!("{source}:{policy_key}")
        };
        let seed_hash = ctx
            .seed_hash
            .unwrap_or_else(|| hash64_fnv1a(stage_key.as_bytes()));

        let model_fallbacks = normalize_string_list(stage.model_fallbacks.clone());
        let targets = self.resolve_stage_targets(&stage, ctx.phase, seed_hash)?;
        if targets.is_empty() {
            return Err(format!("routing stage has no usable targets: {stage_key}"));
        }

        Ok(ResolvedRoutingPlan {
            phase: ctx.phase,
            policy_source: source,
            policy_key: if policy_key.is_empty() {
                None
            } else {
                Some(policy_key)
            },
            targets,
            model_fallbacks,
        })
    }

    pub fn resolve_primary_target(
        &self,
        ctx: RoutingContext<'_>,
    ) -> Result<ResolvedRoutingTarget, String> {
        let mut plan = self.resolve_plan(ctx)?;
        plan.targets.drain(..).next().ok_or_else(|| {
            "routing resolved no target even though stage was expected to have one".to_string()
        })
    }

    fn resolve_stage_targets(
        &self,
        stage: &RoutingStagePolicy,
        phase: RoutingPhase,
        seed_hash: u64,
    ) -> Result<Vec<ResolvedRoutingTarget>, String> {
        if stage.targets.is_empty() {
            return Err("routing stage targets must not be empty".to_string());
        }

        let mut resolved = Vec::<ResolvedRoutingTarget>::new();
        for (idx, target) in stage.targets.iter().enumerate() {
            let profile_key = target.profile.trim();
            if profile_key.is_empty() {
                return Err(format!("routing target[{idx}] has empty profile"));
            }
            let Some((profile_id, profile)) = find_profile(&self.profiles, profile_key) else {
                return Err(format!(
                    "routing target[{idx}] references unknown profile: {profile_key}"
                ));
            };
            let provider_name = profile.provider.trim();
            let mut provider_config = profile.config.clone();

            let model = clean_opt_string(target.model.as_deref())
                .or_else(|| clean_opt_string(provider_config.default_model.as_deref()))
                .ok_or_else(|| {
                    format!(
                        "routing target[{idx}] profile={profile_id} has no model and no default_model"
                    )
                })?;
            provider_config.default_model = Some(model.clone());

            let weight = target.weight.unwrap_or(profile.weight);
            if !weight_valid(weight) {
                return Err(format!(
                    "routing target[{idx}] profile={profile_id} has invalid weight={weight}"
                ));
            }

            validate_runtime_route_for_target(
                idx,
                profile_id,
                provider_name,
                &provider_config,
                model.as_str(),
                phase,
            )?;

            resolved.push(ResolvedRoutingTarget {
                profile: profile_id.to_string(),
                provider: provider_name.to_string(),
                model,
                weight,
                provider_config,
                model_fallbacks: normalize_string_list(target.model_fallbacks.clone()),
            });
        }

        Ok(select_weighted_targets(&resolved, seed_hash))
    }

    fn select_policy(&self, role: Option<&str>, scenario: Option<&str>) -> SelectedPolicy<'_> {
        if let Some((idx, _specificity, rule)) = self.best_matching_override(role, scenario) {
            return SelectedPolicy {
                source: RoutingPolicySource::Override,
                key: format!("override[{idx}]"),
                completion: &rule.completion,
                thinking: rule.thinking.as_ref(),
            };
        }

        if let Some((key, policy)) = find_policy_for_selector(&self.by_scenario, scenario) {
            return SelectedPolicy {
                source: RoutingPolicySource::Scenario,
                key: key.to_string(),
                completion: &policy.completion,
                thinking: policy.thinking.as_ref(),
            };
        }

        if let Some((key, policy)) = find_policy_for_selector(&self.by_role, role) {
            return SelectedPolicy {
                source: RoutingPolicySource::Role,
                key: key.to_string(),
                completion: &policy.completion,
                thinking: policy.thinking.as_ref(),
            };
        }

        SelectedPolicy {
            source: RoutingPolicySource::Default,
            key: String::new(),
            completion: &self.default.completion,
            thinking: self.default.thinking.as_ref(),
        }
    }

    fn best_matching_override(
        &self,
        role: Option<&str>,
        scenario: Option<&str>,
    ) -> Option<(usize, usize, &RoutingOverride)> {
        let mut best: Option<(usize, usize, &RoutingOverride)> = None;
        for (idx, rule) in self.overrides.iter().enumerate() {
            if !selector_matches(rule.role.as_deref(), role) {
                continue;
            }
            if !selector_matches(rule.scenario.as_deref(), scenario) {
                continue;
            }
            let specificity = usize::from(clean_opt_string(rule.role.as_deref()).is_some())
                + usize::from(clean_opt_string(rule.scenario.as_deref()).is_some());
            match best {
                Some((_best_idx, best_specificity, _)) if best_specificity > specificity => {}
                Some((best_idx, best_specificity, _))
                    if best_specificity == specificity && best_idx < idx => {}
                _ => best = Some((idx, specificity, rule)),
            }
        }
        best
    }
}

fn validate_policy(
    policy_path: &str,
    policy: &RoutingPolicy,
    profiles: &BTreeMap<String, RoutingProviderProfile>,
) -> Result<(), String> {
    validate_stage(
        &format!("{policy_path}.completion"),
        &policy.completion,
        profiles,
    )?;
    if let Some(thinking) = policy.thinking.as_ref() {
        validate_stage(&format!("{policy_path}.thinking"), thinking, profiles)?;
    }
    Ok(())
}

fn validate_stage(
    stage_path: &str,
    stage: &RoutingStagePolicy,
    profiles: &BTreeMap<String, RoutingProviderProfile>,
) -> Result<(), String> {
    for (idx, target) in stage.targets.iter().enumerate() {
        let profile = target.profile.trim();
        if profile.is_empty() {
            return Err(format!(
                "{stage_path}.targets[{idx}].profile must not be empty"
            ));
        }
        if find_profile(profiles, profile).is_none() {
            return Err(format!(
                "{stage_path}.targets[{idx}] references unknown profile={profile}"
            ));
        }
        if let Some(model) = target.model.as_deref() {
            if model.trim().is_empty() {
                return Err(format!(
                    "{stage_path}.targets[{idx}].model must not be empty when set"
                ));
            }
        }
        if let Some(weight) = target.weight {
            if !weight_valid(weight) {
                return Err(format!(
                    "{stage_path}.targets[{idx}].weight must be finite and > 0"
                ));
            }
        }
    }
    Ok(())
}

fn validate_runtime_route_for_target(
    idx: usize,
    profile_id: &str,
    provider_name: &str,
    provider_config: &ProviderConfig,
    model: &str,
    phase: RoutingPhase,
) -> Result<(), String> {
    let requirement = routing_phase_requirement(phase);
    let mut errors = Vec::<String>::new();

    for &operation in requirement.preferred_operations {
        match crate::runtime_registry::builtin_runtime_registry_catalog()
            .validate_runtime_route_for_provider_config(
                provider_name,
                provider_config,
                model,
                operation,
                requirement.capability,
            ) {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{operation}: {err}")),
        }
    }

    let operations = requirement
        .preferred_operations
        .iter()
        .map(|operation| operation.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "routing target[{idx}] profile={profile_id} failed catalog runtime resolution for provider={provider_name} model={model} capability={} operations=[{operations}]: {}",
        requirement.capability,
        errors.join("; ")
    ))
}

fn find_profile<'a>(
    profiles: &'a BTreeMap<String, RoutingProviderProfile>,
    key: &str,
) -> Option<(&'a str, &'a RoutingProviderProfile)> {
    if let Some((profile_key, profile)) = profiles
        .iter()
        .find(|(profile_key, _)| profile_key.as_str() == key)
    {
        return Some((profile_key.as_str(), profile));
    }
    let key_norm = key.trim().to_ascii_lowercase();
    profiles
        .iter()
        .find(|(profile_key, _)| profile_key.trim().eq_ignore_ascii_case(&key_norm))
        .map(|(profile_key, profile)| (profile_key.as_str(), profile))
}

fn find_policy_for_selector<'a>(
    policies: &'a BTreeMap<String, RoutingPolicy>,
    selector: Option<&str>,
) -> Option<(&'a str, &'a RoutingPolicy)> {
    if let Some(selector) = clean_opt_string(selector) {
        if let Some((key, policy)) = policies.iter().find(|(key, _)| {
            clean_opt_string(Some(key.as_str()))
                .is_some_and(|normalized| normalized.eq_ignore_ascii_case(&selector))
        }) {
            return Some((key.as_str(), policy));
        }
    }
    policies
        .iter()
        .find(|(key, _)| key.trim() == "*")
        .map(|(key, policy)| (key.as_str(), policy))
}

fn select_weighted_targets(
    candidates: &[ResolvedRoutingTarget],
    seed_hash: u64,
) -> Vec<ResolvedRoutingTarget> {
    let weighted = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| weight_valid(candidate.weight))
        .collect::<Vec<_>>();
    if weighted.is_empty() {
        return Vec::new();
    }

    if weighted.len() == 1 {
        return vec![weighted[0].1.clone()];
    }

    let total_weight: f64 = weighted.iter().map(|(_, candidate)| candidate.weight).sum();
    if !weight_valid(total_weight) {
        return Vec::new();
    }

    let unit = ((seed_hash >> 11) as f64) / ((1u64 << 53) as f64);
    let mut pick = unit * total_weight;
    let mut selected_idx = weighted.len().saturating_sub(1);
    for (idx, (_candidate_idx, candidate)) in weighted.iter().enumerate() {
        if pick < candidate.weight {
            selected_idx = idx;
            break;
        }
        pick -= candidate.weight;
    }

    let mut out = Vec::<ResolvedRoutingTarget>::with_capacity(weighted.len());
    let mut seen = BTreeSet::<String>::new();

    let selected = weighted[selected_idx].1.clone();
    seen.insert(routing_target_key(&selected));
    out.push(selected);

    for (_idx, candidate) in weighted {
        let key = routing_target_key(candidate);
        if seen.insert(key) {
            out.push(candidate.clone());
        }
    }
    out
}

fn routing_target_key(target: &ResolvedRoutingTarget) -> String {
    format!("{}|{}|{}", target.profile, target.provider, target.model)
}

fn clean_opt_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn selector_matches(pattern: Option<&str>, actual: Option<&str>) -> bool {
    let Some(pattern) = clean_opt_string(pattern) else {
        return true;
    };
    if pattern == "*" {
        return true;
    }
    let Some(actual) = clean_opt_string(actual) else {
        return false;
    };
    pattern.eq_ignore_ascii_case(&actual)
}

fn weight_valid(weight: f64) -> bool {
    weight.is_finite() && weight > 0.0
}

fn hash64_fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

impl std::fmt::Display for RoutingPolicySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Role => write!(f, "role"),
            Self::Scenario => write!(f, "scenario"),
            Self::Override => write!(f, "override"),
        }
    }
}

struct SelectedPolicy<'a> {
    source: RoutingPolicySource,
    key: String,
    completion: &'a RoutingStagePolicy,
    thinking: Option<&'a RoutingStagePolicy>,
}

impl<'a> SelectedPolicy<'a> {
    fn stage(&self, phase: RoutingPhase) -> &'a RoutingStagePolicy {
        match phase {
            RoutingPhase::Completion => self.completion,
            RoutingPhase::Thinking => self.thinking.unwrap_or(self.completion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_thinking_and_completion_with_role_scenario_override() {
        let raw = r#"
[profiles.primary]
provider = "compat-primary"
base_url = "https://proxy.example/v1"
weight = 9
default_model = "gpt-4.1"

[profiles.thinking]
provider = "compat-primary"
base_url = "https://proxy.example/v1"
weight = 1
default_model = "o3"

[profiles.backup]
provider = "deepseek"
default_model = "deepseek-chat"

[default.completion]
targets = [{ profile = "primary" }, { profile = "backup", model = "deepseek-reasoner", weight = 2 }]
model_fallbacks = ["gpt-4.1-mini"]

[default.thinking]
targets = [{ profile = "thinking", model = "o3" }]

[by_role.architect.completion]
targets = [{ profile = "primary", model = "gpt-4.1" }]

[by_scenario.long_context.completion]
targets = [{ profile = "backup", model = "deepseek-chat" }]

[[overrides]]
role = "architect"
scenario = "long_context"
[overrides.completion]
targets = [{ profile = "thinking", model = "o3" }]
"#;

        let config = ProviderRoutingConfig::from_toml_str(raw).expect("parse");
        let completion = config
            .resolve_plan(RoutingContext {
                role: Some("architect"),
                scenario: Some("long_context"),
                phase: RoutingPhase::Completion,
                seed_hash: Some(0),
            })
            .expect("completion plan");
        assert_eq!(completion.policy_source, RoutingPolicySource::Override);
        assert_eq!(completion.targets[0].profile, "thinking");
        assert_eq!(completion.targets[0].model, "o3");

        let thinking = config
            .resolve_plan(RoutingContext {
                role: Some("user"),
                scenario: Some("anything"),
                phase: RoutingPhase::Thinking,
                seed_hash: Some(0),
            })
            .expect("thinking plan");
        assert_eq!(thinking.policy_source, RoutingPolicySource::Default);
        assert_eq!(thinking.targets[0].profile, "thinking");
        assert_eq!(thinking.targets[0].model, "o3");
    }

    #[cfg(any(
        feature = "provider-openai",
        feature = "provider-openai-compatible",
        feature = "openai",
        feature = "openai-compatible"
    ))]
    #[test]
    fn weighted_selection_returns_primary_then_fallbacks() {
        let raw = r#"
{
  "profiles": {
    "a": { "provider": "openai-primary", "base_url": "https://proxy.example/v1", "default_model": "gpt-4.1", "weight": 9 },
    "b": { "provider": "openai-primary", "base_url": "https://proxy.example/v1", "default_model": "gpt-4.1-mini", "weight": 1 },
    "c": { "provider": "openai-primary", "base_url": "https://proxy.example/v1", "default_model": "gpt-4.1-nano", "weight": 1 }
  },
  "default": {
    "completion": {
      "targets": [{ "profile": "a" }, { "profile": "b" }, { "profile": "c" }]
    }
  }
}
"#;

        let config = ProviderRoutingConfig::from_json_str(raw).expect("parse");
        let plan = config
            .resolve_plan(RoutingContext {
                role: None,
                scenario: None,
                phase: RoutingPhase::Completion,
                seed_hash: Some(0),
            })
            .expect("resolve");

        assert_eq!(plan.targets.len(), 3);
        assert_eq!(plan.targets[0].profile, "a");
        let profiles = plan
            .targets
            .iter()
            .map(|target| target.profile.clone())
            .collect::<Vec<_>>();
        assert!(profiles.contains(&"a".to_string()));
        assert!(profiles.contains(&"b".to_string()));
        assert!(profiles.contains(&"c".to_string()));
    }

    #[test]
    fn from_path_supports_toml_and_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let toml_path = temp.path().join("routing.toml");
        let json_path = temp.path().join("routing.json");

        let toml_raw = r#"
[profiles.primary]
provider = "compat-primary"
default_model = "gpt-4.1"

[default.completion]
targets = [{ profile = "primary" }]
"#;
        std::fs::write(&toml_path, toml_raw).expect("write toml");

        let json_raw = r#"{
  "profiles": {
    "primary": {
      "provider": "openai-primary",
      "default_model": "gpt-4.1"
    }
  },
  "default": {
    "completion": {
      "targets": [{ "profile": "primary" }]
    }
  }
}"#;
        std::fs::write(&json_path, json_raw).expect("write json");

        let from_toml = ProviderRoutingConfig::from_path(&toml_path).expect("load toml");
        let from_json = ProviderRoutingConfig::from_path(&json_path).expect("load json");

        assert_eq!(from_toml.profiles.len(), 1);
        assert_eq!(from_json.profiles.len(), 1);
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn resolve_plan_rejects_catalog_incompatible_model_for_completion() {
        let raw = r#"
[profiles.embedding_only]
provider = "openai"
default_model = "text-embedding-3-large"

[default.completion]
targets = [{ profile = "embedding_only" }]
"#;

        let config = ProviderRoutingConfig::from_toml_str(raw).expect("parse");
        let err = config
            .resolve_plan(RoutingContext {
                role: None,
                scenario: None,
                phase: RoutingPhase::Completion,
                seed_hash: Some(0),
            })
            .expect_err("embedding-only model should fail llm runtime resolution");
        assert!(err.contains("failed catalog runtime resolution"));
        assert!(err.contains("text-embedding-3-large"));
    }

    #[cfg(any(feature = "provider-openai", feature = "openai"))]
    #[test]
    fn resolve_plan_accepts_response_only_model_for_completion() {
        let raw = r#"
[profiles.response_only]
provider = "openai"
default_model = "computer-use-preview"

[default.completion]
targets = [{ profile = "response_only" }]
"#;

        let config = ProviderRoutingConfig::from_toml_str(raw).expect("parse");
        let plan = config
            .resolve_plan(RoutingContext {
                role: None,
                scenario: None,
                phase: RoutingPhase::Completion,
                seed_hash: Some(0),
            })
            .expect("response-only model should resolve through routing fallback operations");
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].profile, "response_only");
        assert_eq!(plan.targets[0].model, "computer-use-preview");
    }

    #[test]
    fn validate_rejects_unknown_profile_reference() {
        let config = ProviderRoutingConfig {
            profiles: BTreeMap::new(),
            default: RoutingPolicy {
                completion: RoutingStagePolicy {
                    targets: vec![RoutingTarget {
                        profile: "missing".to_string(),
                        model: Some("gpt-4.1".to_string()),
                        weight: None,
                        model_fallbacks: Vec::new(),
                    }],
                    model_fallbacks: Vec::new(),
                },
                thinking: None,
            },
            by_role: BTreeMap::new(),
            by_scenario: BTreeMap::new(),
            overrides: Vec::new(),
        };

        let err = config.validate().expect_err("should fail");
        assert!(err.contains("unknown profile"));
    }
}
