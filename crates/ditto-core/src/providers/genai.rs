use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::contracts::{
    ContentPart, FileSource, FinishReason, ImageSource, Message, Role, Tool, ToolChoice, Usage,
    Warning,
};
use crate::error::Result;

const GOOGLE_TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR: &str = "__gts_";

pub(crate) fn build_google_tool_call_id(seq: u64, thought_signature: Option<&str>) -> String {
    let base_id = format!("call_{seq}");
    let Some(signature) = thought_signature.map(str::trim).filter(|s| !s.is_empty()) else {
        return base_id;
    };
    let mut hex = String::with_capacity(signature.len() * 2);
    for byte in signature.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("{base_id}{GOOGLE_TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR}{hex}")
}

pub(crate) fn extract_google_tool_call_thought_signature(id: &str) -> Option<String> {
    let (_, hex) = id.rsplit_once(GOOGLE_TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR)?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::<u8>::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let hex_pair = std::str::from_utf8(chunk).ok()?;
        let byte = u8::from_str_radix(hex_pair, 16).ok()?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).ok()
}

pub(crate) fn extract_google_part_thought_signature<'a>(
    part: &'a Value,
    function_call: &'a Value,
) -> Option<&'a str> {
    part.get("thoughtSignature")
        .or_else(|| part.get("thought_signature"))
        .or_else(|| function_call.get("thoughtSignature"))
        .or_else(|| function_call.get("thought_signature"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

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

                let text = text.trim();
                if text.is_empty() {
                    continue;
                }

                if system_messages_allowed {
                    system_parts.push(text.to_string());
                } else {
                    warnings.push(Warning::Compatibility {
                        feature: "system_message.mid_conversation".to_string(),
                        details: "Google GenAI only supports systemInstruction at the beginning; downgraded a late system message to user content".to_string(),
                    });
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{ "text": format!("[SYSTEM MESSAGE]\n{text}") }]
                    }));
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
                            id,
                            name,
                            arguments,
                            ..
                        } => {
                            let thought_signature = extract_google_tool_call_thought_signature(id);
                            let mut function_call = Map::<String, Value>::new();
                            function_call.insert("name".to_string(), Value::String(name.clone()));
                            function_call.insert("args".to_string(), arguments.clone());
                            let mut tool_part = Map::<String, Value>::new();
                            tool_part
                                .insert("functionCall".to_string(), Value::Object(function_call));
                            if let Some(thought_signature) = thought_signature {
                                tool_part.insert(
                                    "thoughtSignature".to_string(),
                                    Value::String(thought_signature),
                                );
                            }
                            parts.push(Value::Object(tool_part));
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
    let tool_name = tool.name.clone();
    warn_on_unresolvable_json_schema_refs(&tool_name, &tool.parameters, warnings);
    warn_on_unsupported_json_schema_keywords(&tool_name, &tool.parameters, warnings);
    let mut out = Map::<String, Value>::new();
    out.insert("name".to_string(), Value::String(tool_name.clone()));
    out.insert(
        "description".to_string(),
        Value::String(tool.description.unwrap_or_default()),
    );
    if let Some(parameters) =
        crate::utils::json_schema::convert_json_schema_to_openapi_schema(&tool.parameters, true)
    {
        let mut parameters = parameters;
        let removed = remove_google_unsupported_tool_schema_keywords(&mut parameters);
        if removed > 0 {
            warnings.push(Warning::Compatibility {
                feature: "tool.parameters.google_unsupported_keywords".to_string(),
                details: format!(
                    "tool {tool_name} removed unsupported Google tool schema keyword(s): additionalProperties ({removed} occurrence{})",
                    if removed == 1 { "" } else { "s" }
                ),
            });
        }
        out.insert("parameters".to_string(), parameters);
    }
    Value::Object(out)
}

fn remove_google_unsupported_tool_schema_keywords(value: &mut Value) -> usize {
    match value {
        Value::Object(obj) => {
            let mut removed = usize::from(obj.remove("additionalProperties").is_some());
            for child in obj.values_mut() {
                removed += remove_google_unsupported_tool_schema_keywords(child);
            }
            removed
        }
        Value::Array(values) => values
            .iter_mut()
            .map(remove_google_unsupported_tool_schema_keywords)
            .sum(),
        _ => 0,
    }
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
            "tool {tool_name} uses JSON Schema keywords that are not supported by ditto-core's Google tool schema conversion and will be ignored (e.g. {}{})",
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
        usage.cache_input_tokens = obj
            .get("cachedContentTokenCount")
            .and_then(Value::as_u64)
            .or_else(|| obj.get("cachedTokenCount").and_then(Value::as_u64));
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
            let thought_signature = extract_google_part_thought_signature(part, call);
            let id = build_google_tool_call_id(*tool_call_seq, thought_signature);
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
