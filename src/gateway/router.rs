use serde::{Deserialize, Serialize};

use super::{GatewayError, GatewayRequest, GuardrailsConfig, VirtualKeyConfig};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterConfig {
    pub default_backend: String,
    #[serde(default)]
    pub default_backends: Vec<RouteBackend>,
    pub rules: Vec<RouteRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteBackend {
    pub backend: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
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
        let seed = request.cache_key();
        self.select_backends_for_model_seeded(&request.model, Some(key), Some(&seed))
            .and_then(|backends| {
                backends
                    .into_iter()
                    .next()
                    .ok_or(GatewayError::BackendNotFound {
                        name: "default".to_string(),
                    })
            })
    }

    pub fn select_backend_for_model(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
    ) -> Result<String, GatewayError> {
        self.select_backends_for_model_seeded(model, key, None)
            .and_then(|backends| {
                backends
                    .into_iter()
                    .next()
                    .ok_or(GatewayError::BackendNotFound {
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
        if let Some(key) = key {
            if let Some(route) = &key.route {
                return Ok(vec![route.clone()]);
            }
        }

        for rule in self.config.rules.iter().filter(|rule| rule.exact) {
            if !rule.matches(model) {
                continue;
            }

            let seed = seed.unwrap_or(model);
            if !rule.backends.is_empty() {
                let out = select_weighted(rule.backends.iter(), seed);
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

            let seed = seed.unwrap_or(model);
            if !rule.backends.is_empty() {
                let out = select_weighted(rule.backends.iter(), seed);
                if !out.is_empty() {
                    return Ok(out);
                }
            }

            if !rule.backend.trim().is_empty() {
                return Ok(vec![rule.backend.clone()]);
            }
        }

        let seed = seed.unwrap_or(model);
        if !self.config.default_backends.is_empty() {
            let out = select_weighted(self.config.default_backends.iter(), seed);
            if !out.is_empty() {
                return Ok(out);
            }
        }

        if self.config.default_backend.is_empty() {
            return Err(GatewayError::BackendNotFound {
                name: "default".to_string(),
            });
        }
        Ok(vec![self.config.default_backend.clone()])
    }
}

fn select_weighted<'a>(
    backends: impl Iterator<Item = &'a RouteBackend>,
    seed: &str,
) -> Vec<String> {
    let candidates: Vec<&RouteBackend> = backends
        .filter(|backend| !backend.backend.trim().is_empty())
        .filter(|backend| backend.weight > 0)
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    if candidates.len() == 1 {
        return vec![candidates[0].backend.clone()];
    }

    let total_weight: u64 = candidates.iter().map(|b| u64::from(b.weight)).sum();
    if total_weight == 0 {
        return Vec::new();
    }

    let mut pick = hash64_fnv1a(seed.as_bytes()) % total_weight;
    let mut selected_index = 0usize;
    for (idx, backend) in candidates.iter().enumerate() {
        let weight = u64::from(backend.weight);
        if pick < weight {
            selected_index = idx;
            break;
        }
        pick = pick.saturating_sub(weight);
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

fn hash64_fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_primary(backends: &[RouteBackend], seed: &str) -> String {
        let candidates: Vec<&RouteBackend> = backends
            .iter()
            .filter(|backend| !backend.backend.trim().is_empty())
            .filter(|backend| backend.weight > 0)
            .collect();
        assert!(!candidates.is_empty());
        let total_weight: u64 = candidates.iter().map(|b| u64::from(b.weight)).sum();
        let mut pick = hash64_fnv1a(seed.as_bytes()) % total_weight;
        for backend in candidates {
            let weight = u64::from(backend.weight);
            if pick < weight {
                return backend.backend.clone();
            }
            pick = pick.saturating_sub(weight);
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
                    weight: 1,
                },
                RouteBackend {
                    backend: "b".to_string(),
                    weight: 2,
                },
                RouteBackend {
                    backend: "b".to_string(),
                    weight: 2,
                },
            ],
            guardrails: None,
        };
        let router = Router::new(RouterConfig {
            default_backend: "default".to_string(),
            default_backends: Vec::new(),
            rules: vec![rule.clone()],
        });

        let seed = "req-123";
        let out = router
            .select_backends_for_model_seeded("gpt-4o-mini", None, Some(seed))
            .expect("route");

        assert_eq!(out.len(), 2);
        assert!(out.contains(&"a".to_string()));
        assert!(out.contains(&"b".to_string()));
        assert_eq!(out[0], expected_primary(&rule.backends, seed));
    }

    #[test]
    fn weighted_selection_skips_zero_weight_and_empty_backend() {
        let router = Router::new(RouterConfig {
            default_backend: "default".to_string(),
            default_backends: Vec::new(),
            rules: vec![RouteRule {
                model_prefix: "gpt-".to_string(),
                exact: false,
                backend: String::new(),
                backends: vec![
                    RouteBackend {
                        backend: String::new(),
                        weight: 10,
                    },
                    RouteBackend {
                        backend: "a".to_string(),
                        weight: 0,
                    },
                    RouteBackend {
                        backend: "b".to_string(),
                        weight: 1,
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
            default_backend: "default".to_string(),
            default_backends: Vec::new(),
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
            default_backend: "default".to_string(),
            default_backends: Vec::new(),
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
