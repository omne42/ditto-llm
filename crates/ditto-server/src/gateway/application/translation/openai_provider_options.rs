use serde_json::{Map, Value};

use ditto_core::contracts::GenerateRequest;
use ditto_core::provider_options::{
    ProviderOptions, ProviderOptionsEnvelope, ReasoningEffort, ResponseFormat,
    request_parsed_provider_options,
};

pub(super) fn apply_openai_request_provider_options(
    request: &mut GenerateRequest,
    obj: &Map<String, Value>,
) -> super::ParseResult<()> {
    let mut provider_options = request_parsed_provider_options(request)
        .ok()
        .flatten()
        .unwrap_or_default();
    merge_openai_request_provider_options(&mut provider_options, obj);

    if provider_options != ProviderOptions::default() {
        request.provider_options = Some(
            ProviderOptionsEnvelope::from_options(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(())
}

fn merge_openai_request_provider_options(
    provider_options: &mut ProviderOptions,
    obj: &Map<String, Value>,
) {
    if let Some(reasoning) = obj.get("reasoning").and_then(Value::as_object) {
        if let Some(effort) = reasoning
            .get("effort")
            .and_then(Value::as_str)
            .and_then(parse_reasoning_effort)
        {
            provider_options.reasoning_effort = Some(effort);
        }
    }

    if let Some(parallel) = obj.get("parallel_tool_calls").and_then(Value::as_bool) {
        provider_options.parallel_tool_calls = Some(parallel);
    }

    if let Some(format_value) = obj.get("response_format").and_then(Value::as_object) {
        if let Some(parsed) = parse_json_schema_response_format(format_value) {
            provider_options.response_format = Some(parsed);
        }
    }
}

fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    match value {
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::XHigh),
        _ => None,
    }
}

fn parse_json_schema_response_format(obj: &Map<String, Value>) -> Option<ResponseFormat> {
    let ty = obj.get("type").and_then(Value::as_str)?;
    if ty != "json_schema" {
        return None;
    }
    serde_json::from_value::<ResponseFormat>(Value::Object(obj.clone())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_openai_request_provider_options_to_empty_request() {
        let mut request = GenerateRequest::from(vec![]);
        let obj = serde_json::Map::from_iter([
            (
                "reasoning".to_string(),
                json!({
                    "effort": "high"
                }),
            ),
            ("parallel_tool_calls".to_string(), Value::Bool(false)),
            (
                "response_format".to_string(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "calendar_event",
                        "schema": {
                            "type": "object"
                        },
                        "strict": true
                    }
                }),
            ),
        ]);

        apply_openai_request_provider_options(&mut request, &obj).expect("provider options");

        let parsed = request_parsed_provider_options(&request)
            .expect("parsed provider options")
            .expect("provider options present");
        assert_eq!(parsed.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(parsed.parallel_tool_calls, Some(false));
        assert!(matches!(
            parsed.response_format,
            Some(ResponseFormat::JsonSchema { .. })
        ));
    }

    #[test]
    fn merges_openai_request_provider_options_with_existing_request_options() {
        let mut request = GenerateRequest::from(vec![]);
        request.provider_options = Some(
            ProviderOptionsEnvelope::from_options(ProviderOptions {
                parallel_tool_calls: Some(true),
                ..Default::default()
            })
            .expect("provider options envelope"),
        );
        let obj = serde_json::Map::from_iter([(
            "reasoning".to_string(),
            json!({
                "effort": "medium"
            }),
        )]);

        apply_openai_request_provider_options(&mut request, &obj).expect("provider options");

        let parsed = request_parsed_provider_options(&request)
            .expect("parsed provider options")
            .expect("provider options present");
        assert_eq!(parsed.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(parsed.parallel_tool_calls, Some(true));
    }
}
