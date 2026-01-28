use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::types::{
    ContentPart, FileSource, FinishReason, ImageSource, Message, Role, Tool, ToolChoice, Usage,
    Warning,
};
use crate::{DittoError, Result};

pub(crate) fn build_tool_name_map(messages: &[Message]) -> HashMap<String, String> {
    let mut map = HashMap::<String, String>::new();
    for message in messages {
        for part in &message.content {
            if let ContentPart::ToolCall { id, name, .. } = part {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

pub(crate) fn convert_messages(
    model: &str,
    messages: &[Message],
    tool_names: &HashMap<String, String>,
    warnings: &mut Vec<Warning>,
) -> Result<(Vec<Value>, Option<Value>)> {
    let is_gemma = model.to_lowercase().starts_with("gemma-");
    let mut system_parts = Vec::<String>::new();
    let mut contents = Vec::<Value>::new();
    let mut system_messages_allowed = true;

    for message in messages {
        match message.role {
            Role::System => {
                if !system_messages_allowed {
                    return Err(DittoError::InvalidResponse(
                        "system messages are only supported at the beginning for google provider"
                            .to_string(),
                    ));
                }
                let mut text = String::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text: chunk } => text.push_str(chunk),
                        other => warnings.push(Warning::Unsupported {
                            feature: "system_content_part".to_string(),
                            details: Some(format!("unsupported system content part: {other:?}")),
                        }),
                    }
                }
                if !text.trim().is_empty() {
                    system_parts.push(text);
                }
            }
            Role::User => {
                system_messages_allowed = false;
                let mut parts = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text } => {
                            if !text.is_empty() {
                                parts.push(serde_json::json!({ "text": text }));
                            }
                        }
                        ContentPart::Image { source } => match source {
                            ImageSource::Url { url } => parts.push(serde_json::json!({
                                "fileData": { "mimeType": "image/jpeg", "fileUri": url }
                            })),
                            ImageSource::Base64 { media_type, data } => {
                                parts.push(serde_json::json!({
                                    "inlineData": { "mimeType": media_type, "data": data }
                                }))
                            }
                        },
                        ContentPart::File {
                            media_type,
                            source,
                            ..
                        } => match source {
                            FileSource::Url { url } => parts.push(serde_json::json!({
                                "fileData": { "mimeType": media_type, "fileUri": url }
                            })),
                            FileSource::Base64 { data } => parts.push(serde_json::json!({
                                "inlineData": { "mimeType": media_type, "data": data }
                            })),
                            FileSource::FileId { file_id } => warnings.push(Warning::Unsupported {
                                feature: "file_id".to_string(),
                                details: Some(format!(
                                    "google provider does not support OpenAI file ids (file_id={file_id})"
                                )),
                            }),
                        },
                        other => warnings.push(Warning::Unsupported {
                            feature: "user_content_part".to_string(),
                            details: Some(format!("unsupported user content part: {other:?}")),
                        }),
                    }
                }
                if !parts.is_empty() {
                    contents.push(serde_json::json!({ "role": "user", "parts": parts }));
                }
            }
            Role::Assistant => {
                system_messages_allowed = false;
                let mut parts = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text } => {
                            if !text.is_empty() {
                                parts.push(serde_json::json!({ "text": text }));
                            }
                        }
                        ContentPart::Reasoning { text } => {
                            if !text.is_empty() {
                                parts.push(serde_json::json!({ "text": text, "thought": true }));
                            }
                        }
                        ContentPart::ToolCall {
                            name, arguments, ..
                        } => {
                            parts.push(serde_json::json!({
                                "functionCall": { "name": name, "args": arguments }
                            }));
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "assistant_content_part".to_string(),
                            details: Some(format!("unsupported assistant content part: {other:?}")),
                        }),
                    }
                }
                if !parts.is_empty() {
                    contents.push(serde_json::json!({ "role": "model", "parts": parts }));
                }
            }
            Role::Tool => {
                system_messages_allowed = false;
                let mut parts = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } => {
                            let Some(tool_name) = tool_names.get(tool_call_id) else {
                                warnings.push(Warning::Compatibility {
                                    feature: "tool_result".to_string(),
                                    details: format!(
                                        "tool_result references unknown tool_call_id={tool_call_id}; skipped"
                                    ),
                                });
                                continue;
                            };
                            parts.push(serde_json::json!({
                                "functionResponse": {
                                    "name": tool_name,
                                    "response": { "name": tool_name, "content": content }
                                }
                            }));
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "tool_content_part".to_string(),
                            details: Some(format!("unsupported tool content part: {other:?}")),
                        }),
                    }
                }
                if !parts.is_empty() {
                    contents.push(serde_json::json!({ "role": "user", "parts": parts }));
                }
            }
        }
    }

    if is_gemma && !system_parts.is_empty() {
        if let Some(first) = contents.first_mut() {
            if first.get("role").and_then(Value::as_str) == Some("user") {
                if let Some(parts) = first.get_mut("parts").and_then(Value::as_array_mut) {
                    let system_text = system_parts.join("\n\n") + "\n\n";
                    parts.insert(0, serde_json::json!({ "text": system_text }));
                    system_parts.clear();
                }
            }
        }
    }

    let system_instruction = (!system_parts.is_empty() && !is_gemma).then(|| {
        serde_json::json!({
            "parts": system_parts.iter().map(|t| serde_json::json!({ "text": t })).collect::<Vec<_>>()
        })
    });

    Ok((contents, system_instruction))
}

pub(crate) fn tool_to_google(tool: Tool, warnings: &mut Vec<Warning>) -> Value {
    warn_on_unresolvable_json_schema_refs(&tool.name, &tool.parameters, warnings);
    warn_on_unsupported_json_schema_keywords(&tool.name, &tool.parameters, warnings);
    let mut out = Map::<String, Value>::new();
    out.insert("name".to_string(), Value::String(tool.name));
    out.insert(
        "description".to_string(),
        Value::String(tool.description.unwrap_or_default()),
    );
    if let Some(parameters) =
        crate::utils::json_schema::convert_json_schema_to_openapi_schema(&tool.parameters, true)
    {
        out.insert("parameters".to_string(), parameters);
    }
    Value::Object(out)
}

fn warn_on_unsupported_json_schema_keywords(
    tool_name: &str,
    schema: &Value,
    warnings: &mut Vec<Warning>,
) {
    let keywords = crate::utils::json_schema::collect_unsupported_keywords(schema);
    if keywords.is_empty() {
        return;
    }

    let shown = keywords.iter().take(5).cloned().collect::<Vec<_>>();
    let extra = keywords.len().saturating_sub(shown.len());

    warnings.push(Warning::Compatibility {
        feature: "tool.parameters.unsupported_keywords".to_string(),
        details: format!(
            "tool {tool_name} uses JSON Schema keywords that are not supported by ditto-llm's Google tool schema conversion and will be ignored (e.g. {}{})",
            shown.join(", "),
            if extra == 0 {
                String::new()
            } else {
                format!(", +{extra} more")
            }
        ),
    });
}

fn warn_on_unresolvable_json_schema_refs(
    tool_name: &str,
    schema: &Value,
    warnings: &mut Vec<Warning>,
) {
    fn collect_refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(obj) => {
                if let Some(Value::String(r)) = obj.get("$ref") {
                    if !r.trim().is_empty() {
                        out.push(r.to_string());
                    }
                }
                for v in obj.values() {
                    collect_refs(v, out);
                }
            }
            Value::Array(values) => {
                for v in values {
                    collect_refs(v, out);
                }
            }
            _ => {}
        }
    }

    let mut refs = Vec::<String>::new();
    collect_refs(schema, &mut refs);
    refs.sort();
    refs.dedup();

    let unresolved = refs
        .into_iter()
        .filter(|r| crate::utils::json_schema::resolve_json_schema_ref(schema, r).is_none())
        .collect::<Vec<_>>();

    if unresolved.is_empty() {
        return;
    }

    let shown = unresolved.iter().take(3).cloned().collect::<Vec<_>>();
    let extra = unresolved.len().saturating_sub(shown.len());

    warnings.push(Warning::Compatibility {
        feature: "tool.parameters.$ref".to_string(),
        details: format!(
            "tool {tool_name} uses unresolvable JSON Schema $ref for Google tool schemas; refs will be ignored (e.g. {}{})",
            shown.join(", "),
            if extra == 0 {
                String::new()
            } else {
                format!(", +{extra} more")
            }
        ),
    });
}

pub(crate) fn tool_config(choice: Option<&ToolChoice>) -> Option<Value> {
    let choice = choice?;
    let config = match choice {
        ToolChoice::Auto => serde_json::json!({ "functionCallingConfig": { "mode": "AUTO" } }),
        ToolChoice::None => serde_json::json!({ "functionCallingConfig": { "mode": "NONE" } }),
        ToolChoice::Required => {
            serde_json::json!({ "functionCallingConfig": { "mode": "ANY" } })
        }
        ToolChoice::Tool { name } => serde_json::json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [name] }
        }),
    };
    Some(config)
}

pub(crate) fn map_finish_reason(finish_reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match finish_reason {
        Some("STOP") => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        Some("MAX_TOKENS") => FinishReason::Length,
        Some(
            "IMAGE_SAFETY" | "RECITATION" | "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII",
        ) => FinishReason::ContentFilter,
        Some("MALFORMED_FUNCTION_CALL") => FinishReason::Error,
        _ => FinishReason::Unknown,
    }
}

pub(crate) fn parse_usage_metadata(value: &Value) -> Usage {
    let mut usage = Usage::default();
    if let Some(obj) = value.as_object() {
        usage.input_tokens = obj.get("promptTokenCount").and_then(Value::as_u64);
        usage.output_tokens = obj.get("candidatesTokenCount").and_then(Value::as_u64);
        usage.total_tokens = obj.get("totalTokenCount").and_then(Value::as_u64);
    }
    usage.merge_total();
    usage
}

pub(crate) fn parse_google_candidate(
    candidate: &Value,
    tool_call_seq: &mut u64,
    has_tool_calls: &mut bool,
) -> Vec<ContentPart> {
    let mut out = Vec::<ContentPart>::new();
    let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
    else {
        return out;
    };

    for part in parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                out.push(ContentPart::Text {
                    text: text.to_string(),
                });
            }
            continue;
        }
        if let Some(call) = part.get("functionCall") {
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };
            let args = call.get("args").cloned().unwrap_or(Value::Null);
            let id = format!("call_{}", *tool_call_seq);
            *tool_call_seq = tool_call_seq.saturating_add(1);
            *has_tool_calls = true;
            out.push(ContentPart::ToolCall {
                id,
                name: name.to_string(),
                arguments: args,
            });
        }
    }

    out
}
