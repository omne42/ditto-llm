//! Request adapters for canonical provider-options values.
//!
//! `contracts` owns `GenerateRequest`; this file only provides convenience
//! helpers for reading and writing provider-options envelopes on that DTO.

use serde_json::Value;

use crate::contracts::GenerateRequest;
use crate::error::Result;

use super::{ProviderOptions, ProviderOptionsEnvelope, ResponseFormat};

// PROVIDER-OPTIONS-REQUEST-ADAPTER: request-scoped provider passthrough helpers
// live here as adapters over the contracts-owned request DTO.

pub fn request_with_provider_options(
    mut request: GenerateRequest,
    options: ProviderOptions,
) -> Result<GenerateRequest> {
    request.provider_options = Some(ProviderOptionsEnvelope::from_options(options)?);
    Ok(request)
}

pub fn request_with_provider_response_format(
    mut request: GenerateRequest,
    provider: &str,
    response_format: ResponseFormat,
) -> Result<GenerateRequest> {
    request.provider_options = Some(ProviderOptionsEnvelope::merge_response_format_for_provider(
        request.provider_options.take(),
        provider,
        response_format,
    )?);
    Ok(request)
}

pub fn request_provider_options_for<'a>(
    request: &'a GenerateRequest,
    provider: &str,
) -> Option<&'a Value> {
    request
        .provider_options
        .as_ref()
        .and_then(|options| options.provider_options_for(provider))
}

pub fn request_provider_options_value_for(
    request: &GenerateRequest,
    provider: &str,
) -> Result<Option<Value>> {
    match request.provider_options.as_ref() {
        Some(options) => options.provider_options_value_for(provider),
        None => Ok(None),
    }
}

pub fn request_parsed_provider_options_for(
    request: &GenerateRequest,
    provider: &str,
) -> Result<Option<ProviderOptions>> {
    match request.provider_options.as_ref() {
        Some(options) => options.parsed_provider_options_for(provider),
        None => Ok(None),
    }
}

pub fn request_parsed_provider_options(
    request: &GenerateRequest,
) -> Result<Option<ProviderOptions>> {
    match request.provider_options.as_ref() {
        Some(options) => options.parsed_provider_options(),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DittoError;
    use crate::error::Result;
    use crate::provider_options::{JsonSchemaFormat, ReasoningEffort};
    use serde_json::json;

    #[test]
    fn request_provider_options_roundtrip() -> Result<()> {
        let request = request_with_provider_options(
            GenerateRequest::from(vec![]),
            ProviderOptions {
                reasoning_effort: Some(ReasoningEffort::Medium),
                response_format: Some(ResponseFormat::JsonSchema {
                    json_schema: JsonSchemaFormat {
                        name: "unit_test".to_string(),
                        schema: json!({ "type": "object" }),
                        strict: None,
                    },
                }),
                parallel_tool_calls: Some(false),
                prompt_cache_key: Some("cache_key".to_string()),
            },
        )?;

        let parsed = request_parsed_provider_options(&request)?
            .expect("request should keep provider options");
        assert_eq!(parsed.parallel_tool_calls, Some(false));
        assert_eq!(parsed.prompt_cache_key.as_deref(), Some("cache_key"));
        assert_eq!(parsed.reasoning_effort, Some(ReasoningEffort::Medium));
        Ok(())
    }

    #[test]
    fn request_response_format_merges_into_bucketed_provider_options() -> Result<()> {
        let request = GenerateRequest {
            provider_options: Some(ProviderOptionsEnvelope::from(json!({
                "*": { "parallel_tool_calls": false },
                "openai": { "parallel_tool_calls": true }
            }))),
            ..GenerateRequest::from(vec![])
        };

        let request = request_with_provider_response_format(
            request,
            "openai",
            ResponseFormat::JsonSchema {
                json_schema: JsonSchemaFormat {
                    name: "schema".to_string(),
                    schema: json!({ "type": "object" }),
                    strict: Some(true),
                },
            },
        )?;

        let selected =
            request_provider_options_value_for(&request, "openai")?.expect("bucket should exist");
        let parsed = ProviderOptions::from_value(selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(true));
        assert!(matches!(
            parsed.response_format,
            Some(ResponseFormat::JsonSchema { .. })
        ));
        Ok(())
    }

    #[test]
    fn request_provider_options_rejects_openai_compatible_alias_key() {
        let request = GenerateRequest {
            provider_options: Some(ProviderOptionsEnvelope::from(json!({
                "openai_compatible": { "parallel_tool_calls": true }
            }))),
            ..GenerateRequest::from(vec![])
        };

        let selected = request_provider_options_value_for(&request, "openai-compatible")
            .expect("alias buckets should not error");
        assert!(selected.is_none(), "legacy alias bucket should not resolve");
    }

    #[test]
    fn request_provider_options_supports_bedrock_and_vertex() -> Result<()> {
        let request = GenerateRequest {
            provider_options: Some(ProviderOptionsEnvelope::from(json!({
                "bedrock": { "parallel_tool_calls": true },
                "vertex": { "parallel_tool_calls": false }
            }))),
            ..GenerateRequest::from(vec![])
        };

        let bedrock = request_provider_options_value_for(&request, "bedrock")?
            .expect("bedrock bucket should exist");
        let bedrock = ProviderOptions::from_value(bedrock)?;
        assert_eq!(bedrock.parallel_tool_calls, Some(true));

        let vertex = request_provider_options_value_for(&request, "vertex")?
            .expect("vertex bucket should exist");
        let vertex = ProviderOptions::from_value(vertex)?;
        assert_eq!(vertex.parallel_tool_calls, Some(false));
        Ok(())
    }

    #[test]
    fn request_provider_options_reject_non_object_bucket() {
        let request = GenerateRequest {
            provider_options: Some(ProviderOptionsEnvelope::from(json!({ "*": true }))),
            ..GenerateRequest::from(vec![])
        };

        let err = request_provider_options_value_for(&request, "openai")
            .expect_err("non-object bucket should fail");
        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn request_provider_options_for_returns_none_without_options() {
        let request = GenerateRequest::from(vec![]);
        assert!(request_provider_options_for(&request, "openai").is_none());
    }

    #[test]
    fn request_response_format_rejects_non_object_provider_options() {
        let request = GenerateRequest {
            provider_options: Some(ProviderOptionsEnvelope::from(Value::Bool(true))),
            ..GenerateRequest::from(vec![])
        };

        let err = request_with_provider_response_format(
            request,
            "openai",
            ResponseFormat::JsonSchema {
                json_schema: JsonSchemaFormat {
                    name: "schema".to_string(),
                    schema: json!({ "type": "object" }),
                    strict: None,
                },
            },
        )
        .expect_err("non-object provider_options should fail");

        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
