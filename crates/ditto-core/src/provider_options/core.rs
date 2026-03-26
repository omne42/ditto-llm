//! Canonical provider-options schema core.
//!
//! This layer owns the shared provider-options value schema only. It must stay
//! free of request DTO and warning-surface dependencies so adapters can sit on
//! top without turning the schema itself into a higher-level owner.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JsonSchemaFormat {
    pub name: String,
    pub schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonSchema { json_schema: JsonSchemaFormat },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProviderOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

impl ProviderOptions {
    pub fn from_value(value: Value) -> Result<Self> {
        serde_json::from_value::<Self>(value).map_err(|err| {
            crate::invalid_response!(
                "error_detail.provider_options.invalid",
                "error" => err.to_string()
            )
        })
    }

    pub fn from_value_ref(value: &Value) -> Result<Self> {
        Self::from_value(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use serde_json::json;

    #[test]
    fn provider_options_ignores_unknown_fields() -> Result<()> {
        let parsed = ProviderOptions::from_value(json!({ "unknown": true }))?;
        assert_eq!(parsed, ProviderOptions::default());
        Ok(())
    }
}
