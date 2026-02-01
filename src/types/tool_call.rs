use serde_json::Value;

use crate::types::Warning;

pub(crate) fn parse_tool_call_arguments_json_or_string(
    raw: &str,
    context: &str,
    warnings: &mut Vec<Warning>,
) -> Value {
    let trimmed = raw.trim();
    let raw_json = if trimmed.is_empty() { "{}" } else { trimmed };

    serde_json::from_str::<Value>(raw_json).unwrap_or_else(|err| {
        warnings.push(Warning::Compatibility {
            feature: "tool_call.arguments".to_string(),
            details: format!(
                "failed to parse tool_call arguments as JSON ({context}): {err}; preserving raw string"
            ),
        });
        Value::String(raw.to_string())
    })
}
