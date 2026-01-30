use std::collections::HashMap;

use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct PricingTable {
    models: HashMap<String, ModelPricing>,
}

#[derive(Clone, Debug)]
pub struct ModelPricing {
    pub input_usd_micros_per_token: u64,
    pub output_usd_micros_per_token: u64,
    pub cache_read_input_usd_micros_per_token: Option<u64>,
}

#[derive(Debug, Error)]
pub enum PricingTableError {
    #[error("invalid pricing json: expected object at root")]
    InvalidRoot,
    #[error("invalid pricing entry for model {model}: expected object")]
    InvalidModelEntry { model: String },
    #[error("invalid pricing entry for model {model}: missing both input/output cost")]
    MissingCosts { model: String },
    #[error("invalid pricing entry for model {model}: invalid cost value for {field}")]
    InvalidCostValue { model: String, field: &'static str },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl PricingTable {
    pub fn from_litellm_json_str(raw: &str) -> Result<Self, PricingTableError> {
        let value: serde_json::Value = serde_json::from_str(raw)?;
        Self::from_litellm_json_value(&value)
    }

    pub fn from_litellm_json_value(value: &serde_json::Value) -> Result<Self, PricingTableError> {
        let Some(root) = value.as_object() else {
            return Err(PricingTableError::InvalidRoot);
        };

        let mut models = HashMap::new();
        for (model, entry) in root {
            let Some(obj) = entry.as_object() else {
                return Err(PricingTableError::InvalidModelEntry {
                    model: model.clone(),
                });
            };

            let input = parse_cost_usd_per_token(obj, "input_cost_per_token")
                .or_else(|| parse_cost_usd_per_1k_tokens(obj, "input_cost_per_1k_tokens"))
                .map(|usd| usd_to_usd_micros_per_token(usd, model, "input_cost"))
                .transpose()?;

            let output = parse_cost_usd_per_token(obj, "output_cost_per_token")
                .or_else(|| parse_cost_usd_per_1k_tokens(obj, "output_cost_per_1k_tokens"))
                .map(|usd| usd_to_usd_micros_per_token(usd, model, "output_cost"))
                .transpose()?;

            let cache_read_input = parse_cost_usd_per_token(obj, "cache_read_input_token_cost")
                .map(|usd| usd_to_usd_micros_per_token(usd, model, "cache_read_input_cost"))
                .transpose()?;

            let Some(input_usd_micros_per_token) = input else {
                if output.is_some() {
                    models.insert(
                        model.clone(),
                        ModelPricing {
                            input_usd_micros_per_token: 0,
                            output_usd_micros_per_token: output.unwrap_or(0),
                            cache_read_input_usd_micros_per_token: cache_read_input,
                        },
                    );
                    continue;
                }
                return Err(PricingTableError::MissingCosts {
                    model: model.clone(),
                });
            };

            models.insert(
                model.clone(),
                ModelPricing {
                    input_usd_micros_per_token,
                    output_usd_micros_per_token: output.unwrap_or(0),
                    cache_read_input_usd_micros_per_token: cache_read_input,
                },
            );
        }

        Ok(Self { models })
    }

    pub fn model_pricing(&self, model: &str) -> Option<&ModelPricing> {
        self.models.get(model)
    }

    pub fn estimate_cost_usd_micros(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Option<u64> {
        self.estimate_cost_usd_micros_with_cache(model, input_tokens, None, output_tokens)
    }

    pub fn estimate_cost_usd_micros_with_cache(
        &self,
        model: &str,
        input_tokens: u32,
        cache_input_tokens: Option<u32>,
        output_tokens: u32,
    ) -> Option<u64> {
        let pricing = self.model_pricing(model)?;

        let cached_tokens = cache_input_tokens.unwrap_or(0);
        let cached_tokens = std::cmp::min(cached_tokens, input_tokens);

        let input_cost = if cached_tokens > 0 {
            match pricing.cache_read_input_usd_micros_per_token {
                Some(cache_read_cost) => {
                    let non_cached_tokens = input_tokens.saturating_sub(cached_tokens);
                    let non_cached = u64::from(non_cached_tokens)
                        .saturating_mul(pricing.input_usd_micros_per_token);
                    let cached = u64::from(cached_tokens).saturating_mul(cache_read_cost);
                    non_cached.saturating_add(cached)
                }
                None => u64::from(input_tokens).saturating_mul(pricing.input_usd_micros_per_token),
            }
        } else {
            u64::from(input_tokens).saturating_mul(pricing.input_usd_micros_per_token)
        };

        let output_cost =
            u64::from(output_tokens).saturating_mul(pricing.output_usd_micros_per_token);
        Some(input_cost.saturating_add(output_cost))
    }
}

fn parse_cost_usd_per_token(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Option<f64> {
    obj.get(key).and_then(|value| value.as_f64())
}

fn parse_cost_usd_per_1k_tokens(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Option<f64> {
    let per_1k = obj.get(key).and_then(|value| value.as_f64())?;
    Some(per_1k / 1000.0)
}

fn usd_to_usd_micros_per_token(
    usd_per_token: f64,
    model: &str,
    field: &'static str,
) -> Result<u64, PricingTableError> {
    if !usd_per_token.is_finite() || usd_per_token < 0.0 {
        return Err(PricingTableError::InvalidCostValue {
            model: model.to_string(),
            field,
        });
    }
    let micros = (usd_per_token * 1_000_000.0).round();
    if !micros.is_finite() || micros < 0.0 {
        return Err(PricingTableError::InvalidCostValue {
            model: model.to_string(),
            field,
        });
    }
    let micros = if micros > u64::MAX as f64 {
        u64::MAX
    } else {
        micros as u64
    };
    Ok(micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_litellm_pricing_json() {
        let raw = r#"{
          "gpt-4o-mini": {"input_cost_per_token": 0.000001, "output_cost_per_token": 0.000002},
          "o1": {"input_cost_per_1k_tokens": 1.0, "output_cost_per_1k_tokens": 2.0},
          "claude-3-5-haiku-20241022": {"input_cost_per_token": 0.000002, "output_cost_per_token": 0.000004, "cache_read_input_token_cost": 0.000001}
        }"#;
        let table = PricingTable::from_litellm_json_str(raw).expect("pricing");
        let pricing = table.model_pricing("gpt-4o-mini").expect("pricing");
        assert_eq!(pricing.input_usd_micros_per_token, 1);
        assert_eq!(pricing.output_usd_micros_per_token, 2);
        assert_eq!(pricing.cache_read_input_usd_micros_per_token, None);

        let o1 = table.model_pricing("o1").expect("o1");
        assert_eq!(o1.input_usd_micros_per_token, 1000);
        assert_eq!(o1.output_usd_micros_per_token, 2000);
        assert_eq!(o1.cache_read_input_usd_micros_per_token, None);

        let claude = table
            .model_pricing("claude-3-5-haiku-20241022")
            .expect("claude");
        assert_eq!(claude.input_usd_micros_per_token, 2);
        assert_eq!(claude.output_usd_micros_per_token, 4);
        assert_eq!(claude.cache_read_input_usd_micros_per_token, Some(1));

        let cost = table
            .estimate_cost_usd_micros("gpt-4o-mini", 3, 4)
            .expect("cost");
        assert_eq!(cost, 3 + 8);

        let cost_cached = table
            .estimate_cost_usd_micros_with_cache("claude-3-5-haiku-20241022", 10, Some(4), 1)
            .expect("cost cached");
        assert_eq!(cost_cached, 12 + 4 + 4);
    }
}
