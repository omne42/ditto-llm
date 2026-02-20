use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest, GuardrailsConfig, VirtualKeyConfig};

#[derive(Clone, Debug, Serialize)]
pub struct RouterConfig {
    #[serde(default)]
    pub default_backends: Vec<RouteBackend>,
    pub rules: Vec<RouteRule>,
}

impl<'de> Deserialize<'de> for RouterConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RouterConfigCompat {
            #[serde(default)]
            default_backends: Vec<RouteBackend>,
            #[serde(default)]
            rules: Vec<RouteRule>,
            #[serde(default)]
            default_backend: Option<String>,
        }

        let mut compat = RouterConfigCompat::deserialize(deserializer)?;

        let legacy_default = compat
            .default_backend
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if legacy_default.is_some() && !compat.default_backends.is_empty() {
            return Err(de::Error::custom(
                "router.default_backend is deprecated; use router.default_backends only",
            ));
        }

        if compat.default_backends.is_empty() {
            if let Some(default_backend) = legacy_default {
                compat.default_backends.push(RouteBackend {
                    backend: default_backend.to_string(),
                    weight: default_weight(),
                });
            }
        }

        Ok(Self {
            default_backends: compat.default_backends,
            rules: compat.rules,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteBackend {
    pub backend: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteRule {
    pub model_prefix: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact: bool,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub backends: Vec<RouteBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<GuardrailsConfig>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl RouteRule {
    pub fn matches(&self, model: &str) -> bool {
        if self.exact {
            model == self.model_prefix
        } else {
            let prefix = self
                .model_prefix
                .strip_suffix('*')
                .unwrap_or(&self.model_prefix);
            model.starts_with(prefix)
        }
    }
}

#[derive(Debug)]
pub struct Router {
    config: RouterConfig,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        Self { config }
    }

    pub fn rule_for_model(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
    ) -> Option<&RouteRule> {
        if let Some(key) = key {
            if key.route.is_some() {
                return None;
            }
        }
        let exact = self
            .config
            .rules
            .iter()
            .find(|rule| rule.exact && rule.matches(model));
        if exact.is_some() {
            return exact;
        }
        self.config
            .rules
            .iter()
            .find(|rule| !rule.exact && rule.matches(model))
    }

    pub fn select_backend(
        &self,
        request: &GatewayRequest,
        key: &VirtualKeyConfig,
    ) -> Result<String, GatewayError> {
        let seed_hash = request.route_seed_hash(&key.id);
        self.select_backends_for_model_seeded_hash(&request.model, Some(key), Some(seed_hash))
            .and_then(|backends| {
                backends
                    .into_iter()
                    .next()
                    .ok_or_else(|| GatewayError::BackendNotFound {
                        name: "default".to_string(),
                    })
            })
    }

    pub fn select_backend_for_model(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
    ) -> Result<String, GatewayError> {
        self.select_backends_for_model_seeded_hash(model, key, None)
            .and_then(|backends| {
                backends
                    .into_iter()
                    .next()
                    .ok_or_else(|| GatewayError::BackendNotFound {
                        name: "default".to_string(),
                    })
            })
    }

    pub fn select_backends_for_model_seeded(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
        seed: Option<&str>,
    ) -> Result<Vec<String>, GatewayError> {
        let seed_hash = seed.map(|seed| super::hash64_fnv1a(seed.as_bytes()));
        self.select_backends_for_model_seeded_hash(model, key, seed_hash)
    }

    pub fn select_backends_for_model_seeded_hash(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
        seed_hash: Option<u64>,
    ) -> Result<Vec<String>, GatewayError> {
        if let Some(key) = key {
            if let Some(route) = &key.route {
                return Ok(vec![route.clone()]);
            }
        }

        let seed_hash = seed_hash.unwrap_or_else(|| super::hash64_fnv1a(model.as_bytes()));

        for rule in self.config.rules.iter().filter(|rule| rule.exact) {
            if !rule.matches(model) {
                continue;
            }

            if !rule.backends.is_empty() {
                let out = select_weighted(rule.backends.iter(), seed_hash);
                if !out.is_empty() {
                    return Ok(out);
                }
            }

            if !rule.backend.trim().is_empty() {
                return Ok(vec![rule.backend.clone()]);
            }
        }

        for rule in self.config.rules.iter().filter(|rule| !rule.exact) {
            if !rule.matches(model) {
                continue;
            }

            if !rule.backends.is_empty() {
                let out = select_weighted(rule.backends.iter(), seed_hash);
                if !out.is_empty() {
                    return Ok(out);
                }
            }

            if !rule.backend.trim().is_empty() {
                return Ok(vec![rule.backend.clone()]);
            }
        }

        if !self.config.default_backends.is_empty() {
            let out = select_weighted(self.config.default_backends.iter(), seed_hash);
            if !out.is_empty() {
                return Ok(out);
            }
        }
        Err(GatewayError::BackendNotFound {
            name: "default".to_string(),
        })
    }
}

fn select_weighted<'a>(
    backends: impl Iterator<Item = &'a RouteBackend>,
    seed_hash: u64,
) -> Vec<String> {
    let candidates: Vec<&RouteBackend> = backends
        .filter(|backend| !backend.backend.trim().is_empty())
        .filter(|backend| backend.weight.is_finite() && backend.weight > 0.0)
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    if candidates.len() == 1 {
        return vec![candidates[0].backend.clone()];
    }

    let total_weight: f64 = candidates.iter().map(|b| b.weight).sum();
    if !total_weight.is_finite() || total_weight <= 0.0 {
        return Vec::new();
    }

    let unit = ((seed_hash >> 11) as f64) / ((1u64 << 53) as f64);
    let mut pick = unit * total_weight;
    let mut selected_index = candidates.len().saturating_sub(1);
    for (idx, backend) in candidates.iter().enumerate() {
        let weight = backend.weight;
        if pick < weight {
            selected_index = idx;
            break;
        }
        pick -= weight;
    }

    let mut out = Vec::with_capacity(candidates.len());
    out.push(candidates[selected_index].backend.clone());

    for (idx, backend) in candidates.iter().enumerate() {
        if idx == selected_index {
            continue;
        }
        if out
            .iter()
            .any(|existing| existing == backend.backend.as_str())
        {
            continue;
        }
        out.push(backend.backend.clone());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_primary(backends: &[RouteBackend], seed_hash: u64) -> String {
        let candidates: Vec<&RouteBackend> = backends
            .iter()
            .filter(|backend| !backend.backend.trim().is_empty())
            .filter(|backend| backend.weight.is_finite() && backend.weight > 0.0)
            .collect();
        assert!(!candidates.is_empty());
        let total_weight: f64 = candidates.iter().map(|b| b.weight).sum();
        let unit = ((seed_hash >> 11) as f64) / ((1u64 << 53) as f64);
        let mut pick = unit * total_weight;
        for backend in candidates {
            let weight = backend.weight;
            if pick < weight {
                return backend.backend.clone();
            }
            pick -= weight;
        }
        unreachable!("weight selection must pick an element")
    }

    #[test]
    fn weighted_selection_is_deterministic_and_dedups() {
        let rule = RouteRule {
            model_prefix: "gpt-".to_string(),
            exact: false,
            backend: String::new(),
            backends: vec![
                RouteBackend {
                    backend: "a".to_string(),
                    weight: 1.0,
                },
                RouteBackend {
                    backend: "b".to_string(),
                    weight: 2.0,
                },
                RouteBackend {
                    backend: "b".to_string(),
                    weight: 2.0,
                },
            ],
            guardrails: None,
        };
        let router = Router::new(RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "default".to_string(),
                weight: 1.0,
            }],
            rules: vec![rule.clone()],
        });

        let seed = "req-123";
        let seed_hash = super::super::hash64_fnv1a(seed.as_bytes());
        let out = router
            .select_backends_for_model_seeded("gpt-4o-mini", None, Some(seed))
            .expect("route");

        assert_eq!(out.len(), 2);
        assert!(out.contains(&"a".to_string()));
        assert!(out.contains(&"b".to_string()));
        assert_eq!(out[0], expected_primary(&rule.backends, seed_hash));
    }

    #[test]
    fn weighted_selection_skips_zero_weight_and_empty_backend() {
        let router = Router::new(RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "default".to_string(),
                weight: 1.0,
            }],
            rules: vec![RouteRule {
                model_prefix: "gpt-".to_string(),
                exact: false,
                backend: String::new(),
                backends: vec![
                    RouteBackend {
                        backend: String::new(),
                        weight: 10.0,
                    },
                    RouteBackend {
                        backend: "a".to_string(),
                        weight: 0.0,
                    },
                    RouteBackend {
                        backend: "b".to_string(),
                        weight: 1.0,
                    },
                ],
                guardrails: None,
            }],
        });

        let out = router
            .select_backends_for_model_seeded("gpt-4o-mini", None, Some("seed"))
            .expect("route");
        assert_eq!(out, vec!["b".to_string()]);
    }

    #[test]
    fn exact_rules_take_precedence_over_prefix_rules() {
        let router = Router::new(RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "default".to_string(),
                weight: 1.0,
            }],
            rules: vec![
                RouteRule {
                    model_prefix: "gpt-".to_string(),
                    exact: false,
                    backend: "prefix".to_string(),
                    backends: Vec::new(),
                    guardrails: None,
                },
                RouteRule {
                    model_prefix: "gpt-4o-mini".to_string(),
                    exact: true,
                    backend: "exact".to_string(),
                    backends: Vec::new(),
                    guardrails: None,
                },
            ],
        });

        let out = router
            .select_backend_for_model("gpt-4o-mini", None)
            .expect("route");
        assert_eq!(out, "exact".to_string());
    }

    #[test]
    fn wildcard_suffix_matches_as_prefix() {
        let router = Router::new(RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "default".to_string(),
                weight: 1.0,
            }],
            rules: vec![RouteRule {
                model_prefix: "anthropic/*".to_string(),
                exact: false,
                backend: "primary".to_string(),
                backends: Vec::new(),
                guardrails: None,
            }],
        });

        let out = router
            .select_backend_for_model("anthropic/claude-3-opus", None)
            .expect("route");
        assert_eq!(out, "primary".to_string());
    }
}
