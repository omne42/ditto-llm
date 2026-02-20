use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OpenAiUsage {
    pub(crate) prompt_tokens: Option<u64>,
    pub(crate) completion_tokens: Option<u64>,
    pub(crate) total_tokens: Option<u64>,
}

pub(crate) fn parse_openai_usage(value: &Value) -> OpenAiUsage {
    let mut usage = OpenAiUsage::default();
    let Some(obj) = value.as_object() else {
        return usage;
    };
    usage.prompt_tokens = obj
        .get("prompt_tokens")
        .or_else(|| obj.get("input_tokens"))
        .and_then(Value::as_u64);
    usage.completion_tokens = obj
        .get("completion_tokens")
        .or_else(|| obj.get("output_tokens"))
        .and_then(Value::as_u64);
    usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64).or_else(|| {
        usage.prompt_tokens.and_then(|prompt| {
            usage
                .completion_tokens
                .map(|completion| prompt.saturating_add(completion))
        })
    });
    usage
}

fn number_from_f64(value: f64) -> Option<Value> {
    serde_json::Number::from_f64(value).map(Value::Number)
}

fn extract_text_from_blocks(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_object())
            .filter_map(|obj| match obj.get("type").and_then(Value::as_str) {
                Some("text") => obj.get("text").and_then(Value::as_str),
                _ => None,
            })
            .collect(),
        _ => String::new(),
    }
}

fn anthropic_tool_choice_to_openai(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("auto") => Some(Value::String("auto".to_string())),
        Some("any") => Some(Value::String("required".to_string())),
        Some("tool") => {
            let name = obj.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "type": "function",
                "function": { "name": name }
            }))
        }
        _ => None,
    }
}

fn anthropic_tools_to_openai(value: &Value) -> Result<Vec<Value>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "tools must be an array".to_string())?;

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "tool missing name".to_string())?;
        let description = obj.get("description").and_then(Value::as_str);
        let parameters = obj
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "type": "object" }));
        let mut function = Map::<String, Value>::new();
        function.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(description) = description {
            if !description.trim().is_empty() {
                function.insert(
                    "description".to_string(),
                    Value::String(description.to_string()),
                );
            }
        }
        function.insert("parameters".to_string(), parameters);
        out.push(serde_json::json!({
            "type": "function",
            "function": Value::Object(function),
        }));
    }
    Ok(out)
}

fn push_openai_user_message(messages: &mut Vec<Value>, parts: &mut Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    let content = Value::Array(std::mem::take(parts));
    messages.push(serde_json::json!({
        "role": "user",
        "content": content,
    }));
}

pub(crate) fn anthropic_messages_request_to_openai_chat_completions(
    request: &Value,
) -> Result<Value, String> {
    let obj = request
        .as_object()
        .ok_or_else(|| "request body must be a JSON object".to_string())?;

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing field `model`".to_string())?;

    let stream = obj.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let max_tokens = obj.get("max_tokens").and_then(Value::as_u64);
    let temperature = obj.get("temperature").and_then(Value::as_f64);
    let top_p = obj.get("top_p").and_then(Value::as_f64);
    let stop_sequences = obj
        .get("stop_sequences")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let system = obj
        .get("system")
        .map(extract_text_from_blocks)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut messages = Vec::<Value>::new();
    if let Some(system) = system {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system,
        }));
    }

    let items = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing field `messages`".to_string())?;

    for item in items {
        let Some(msg) = item.as_object() else {
            continue;
        };
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let content = msg.get("content").unwrap_or(&Value::Null);

        match role.as_str() {
            "user" => {
                let mut parts = Vec::<Value>::new();
                let synthesized_block;
                let blocks: &[Value] = match content {
                    Value::String(text) => {
                        synthesized_block = serde_json::json!({ "type": "text", "text": text });
                        std::slice::from_ref(&synthesized_block)
                    }
                    Value::Array(items) => items.as_slice(),
                    other => std::slice::from_ref(other),
                };

                for block in blocks {
                    let Some(block) = block.as_object() else {
                        continue;
                    };
                    let kind = block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match kind {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                if !text.is_empty() {
                                    parts.push(serde_json::json!({
                                        "type": "text",
                                        "text": text,
                                    }));
                                }
                            }
                        }
                        "image" => {
                            let Some(source) = block.get("source").and_then(Value::as_object)
                            else {
                                continue;
                            };
                            let url = match source.get("type").and_then(Value::as_str) {
                                Some("url") => source
                                    .get("url")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(|s| s.to_string()),
                                Some("base64") => {
                                    let media_type = source
                                        .get("media_type")
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty());
                                    let data = source
                                        .get("data")
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty());
                                    match (media_type, data) {
                                        (Some(media_type), Some(data)) => {
                                            Some(format!("data:{media_type};base64,{data}"))
                                        }
                                        _ => None,
                                    }
                                }
                                _ => None,
                            };

                            if let Some(url) = url {
                                parts.push(serde_json::json!({
                                    "type": "image_url",
                                    "image_url": { "url": url },
                                }));
                            }
                        }
                        "tool_result" => {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|s| !s.is_empty());
                            let tool_use_id = match tool_use_id {
                                Some(id) => id.to_string(),
                                None => continue,
                            };
                            let tool_content = block.get("content").unwrap_or(&Value::Null);
                            let tool_text = extract_text_from_blocks(tool_content);

                            push_openai_user_message(&mut messages, &mut parts);
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": tool_text,
                            }));
                        }
                        _ => {}
                    }
                }

                push_openai_user_message(&mut messages, &mut parts);
            }
            "assistant" => {
                let synthesized_block;
                let blocks: &[Value] = match content {
                    Value::String(text) => {
                        synthesized_block = serde_json::json!({ "type": "text", "text": text });
                        std::slice::from_ref(&synthesized_block)
                    }
                    Value::Array(items) => items.as_slice(),
                    other => std::slice::from_ref(other),
                };

                let mut text = String::new();
                let mut tool_calls = Vec::<Value>::new();
                for (idx, block) in blocks.iter().enumerate() {
                    let Some(block) = block.as_object() else {
                        continue;
                    };
                    let kind = block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match kind {
                        "text" => {
                            if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                                text.push_str(chunk);
                            }
                        }
                        "tool_use" => {
                            let id = block
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| format!("call_{idx}"));
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                            if name.is_empty() {
                                continue;
                            }
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string(),
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut message = Map::<String, Value>::new();
                message.insert("role".to_string(), Value::String("assistant".to_string()));
                if !text.is_empty() {
                    message.insert("content".to_string(), Value::String(text));
                } else {
                    message.insert("content".to_string(), Value::Null);
                }
                if !tool_calls.is_empty() {
                    message.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                messages.push(Value::Object(message));
            }
            _ => {}
        }
    }

    let mut out = Map::<String, Value>::new();
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("messages".to_string(), Value::Array(messages));
    out.insert("stream".to_string(), Value::Bool(stream));
    if let Some(max_tokens) = max_tokens {
        out.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    }
    if let Some(temperature) = temperature.and_then(number_from_f64) {
        out.insert("temperature".to_string(), temperature);
    }
    if let Some(top_p) = top_p.and_then(number_from_f64) {
        out.insert("top_p".to_string(), top_p);
    }
    if !stop_sequences.is_empty() {
        out.insert(
            "stop".to_string(),
            Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
        );
    }

    if let Some(tools) = obj.get("tools") {
        let mapped = anthropic_tools_to_openai(tools)?;
        if !mapped.is_empty() {
            out.insert("tools".to_string(), Value::Array(mapped));
        }
    }
    if let Some(tool_choice) = obj
        .get("tool_choice")
        .and_then(anthropic_tool_choice_to_openai)
    {
        out.insert("tool_choice".to_string(), tool_choice);
    }

    Ok(Value::Object(out))
}

fn openai_finish_reason_to_anthropic(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some("stop") | None => Some("end_turn"),
        Some("length") => Some("max_tokens"),
        Some("tool_calls") | Some("function_call") => Some("tool_use"),
        Some("content_filter") => Some("content_filtered"),
        Some(_) => Some("end_turn"),
    }
}

pub(crate) fn openai_chat_completions_response_to_anthropic_message(
    response: &Value,
) -> Result<Value, String> {
    let obj = response
        .as_object()
        .ok_or_else(|| "openai response must be a JSON object".to_string())?;
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl_unknown")
        .to_string();
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let choice = obj
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
        .ok_or_else(|| "openai response missing choices[0]".to_string())?;

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let stop_reason = openai_finish_reason_to_anthropic(finish_reason);

    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "openai response missing choices[0].message".to_string())?;

    let mut content_blocks = Vec::<Value>::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content_blocks.push(serde_json::json!({
                "type": "text",
                "text": text,
            }));
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (idx, call) in tool_calls.iter().enumerate() {
            let Some(call) = call.as_object() else {
                continue;
            };
            let call_id = call.get("id").and_then(Value::as_str).unwrap_or("").trim();
            let call_id = if call_id.is_empty() {
                format!("call_{idx}")
            } else {
                call_id.to_string()
            };

            let function = call
                .get("function")
                .and_then(Value::as_object)
                .unwrap_or(call);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let args_raw = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(args_raw)
                .unwrap_or_else(|_| Value::Object(Map::new()));
            content_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": input,
            }));
        }
    }

    let usage = obj.get("usage").map(parse_openai_usage).unwrap_or_default();
    let usage_obj = serde_json::json!({
        "input_tokens": usage.prompt_tokens.unwrap_or(0),
        "output_tokens": usage.completion_tokens.unwrap_or(0),
    });

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id));
    out.insert("type".to_string(), Value::String("message".to_string()));
    out.insert("role".to_string(), Value::String("assistant".to_string()));
    if !model.trim().is_empty() {
        out.insert("model".to_string(), Value::String(model));
    }
    out.insert("content".to_string(), Value::Array(content_blocks));
    if let Some(stop_reason) = stop_reason {
        out.insert(
            "stop_reason".to_string(),
            Value::String(stop_reason.to_string()),
        );
    } else {
        out.insert("stop_reason".to_string(), Value::Null);
    }
    out.insert("stop_sequence".to_string(), Value::Null);
    out.insert("usage".to_string(), usage_obj);
    Ok(Value::Object(out))
}

fn google_tool_choice_to_openai(value: &Value) -> Option<Value> {
    let config = value
        .as_object()?
        .get("functionCallingConfig")?
        .as_object()?;
    let mode = config
        .get("mode")
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_uppercase();
    match mode.as_str() {
        "AUTO" => Some(Value::String("auto".to_string())),
        "NONE" => Some(Value::String("none".to_string())),
        "ANY" => {
            let allowed = config.get("allowedFunctionNames").and_then(Value::as_array);
            let allowed = allowed.and_then(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .find(|s| !s.is_empty())
                    .map(|s| s.to_string())
            });
            match allowed {
                Some(name) => Some(serde_json::json!({
                    "type": "function",
                    "function": { "name": name }
                })),
                None => Some(Value::String("required".to_string())),
            }
        }
        _ => None,
    }
}

fn google_tools_to_openai(value: &Value) -> Result<Vec<Value>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "tools must be an array".to_string())?;
    let mut out = Vec::<Value>::new();
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(decls) = obj.get("functionDeclarations").and_then(Value::as_array) else {
            continue;
        };
        for decl in decls {
            let Some(decl) = decl.as_object() else {
                continue;
            };
            let name = decl
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "tool missing functionDeclarations[].name".to_string())?;
            let description = decl.get("description").and_then(Value::as_str);
            let parameters = decl.get("parameters").cloned();

            let mut function = Map::<String, Value>::new();
            function.insert("name".to_string(), Value::String(name.to_string()));
            if let Some(description) = description {
                if !description.trim().is_empty() {
                    function.insert(
                        "description".to_string(),
                        Value::String(description.to_string()),
                    );
                }
            }
            if let Some(parameters) = parameters {
                function.insert("parameters".to_string(), parameters);
            }

            out.push(serde_json::json!({
                "type": "function",
                "function": Value::Object(function),
            }));
        }
    }
    Ok(out)
}

pub(crate) fn google_generate_content_request_to_openai_chat_completions(
    model: &str,
    request: &Value,
    stream: bool,
) -> Result<Value, String> {
    let obj = request
        .as_object()
        .ok_or_else(|| "request body must be a JSON object".to_string())?;

    let mut messages = Vec::<Value>::new();
    if let Some(system) = obj.get("system_instruction").and_then(Value::as_object) {
        if let Some(parts) = system.get("parts").and_then(Value::as_array) {
            let text = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<String>();
            if !text.trim().is_empty() {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": text,
                }));
            }
        }
    }

    if let Some(contents) = obj.get("contents").and_then(Value::as_array) {
        for item in contents {
            let Some(content) = item.as_object() else {
                continue;
            };
            let role = content
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mapped_role = match role {
                "user" => "user",
                "model" => "assistant",
                _ => continue,
            };
            let Some(parts) = content.get("parts").and_then(Value::as_array) else {
                continue;
            };
            let text = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<String>();
            if text.trim().is_empty() {
                continue;
            }
            messages.push(serde_json::json!({
                "role": mapped_role,
                "content": text,
            }));
        }
    }

    let mut out = Map::<String, Value>::new();
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("messages".to_string(), Value::Array(messages));
    out.insert("stream".to_string(), Value::Bool(stream));

    if let Some(cfg) = obj.get("generationConfig").and_then(Value::as_object) {
        if let Some(max_tokens) = cfg.get("maxOutputTokens").and_then(Value::as_u64) {
            out.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
        }
        if let Some(temperature) = cfg.get("temperature").and_then(Value::as_f64) {
            if let Some(value) = number_from_f64(temperature) {
                out.insert("temperature".to_string(), value);
            }
        }
        if let Some(top_p) = cfg.get("topP").and_then(Value::as_f64) {
            if let Some(value) = number_from_f64(top_p) {
                out.insert("top_p".to_string(), value);
            }
        }
        if let Some(stop) = cfg.get("stopSequences").and_then(Value::as_array) {
            let stops = stop
                .iter()
                .filter_map(Value::as_str)
                .map(|s| Value::String(s.to_string()))
                .collect::<Vec<_>>();
            if !stops.is_empty() {
                out.insert("stop".to_string(), Value::Array(stops));
            }
        }
    }

    if let Some(tools) = obj.get("tools") {
        let tools = google_tools_to_openai(tools)?;
        if !tools.is_empty() {
            out.insert("tools".to_string(), Value::Array(tools));
        }
    }
    if let Some(tool_choice) = obj.get("toolConfig").and_then(google_tool_choice_to_openai) {
        out.insert("tool_choice".to_string(), tool_choice);
    }

    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_text_from_blocks_joins_text_parts_only() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "image", "source": {"type": "url", "url": "https://example.com/a.png"}},
            {"type": "text", "text": " world"}
        ]);
        assert_eq!(extract_text_from_blocks(&value), "hello world");
    }

    #[test]
    fn anthropic_response_assigns_unique_fallback_tool_use_ids() {
        let response = json!({
            "id": "chatcmpl_123",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {"function": {"name": "tool_a", "arguments": "{\"a\":1}"}},
                        {"function": {"name": "tool_b", "arguments": "{\"b\":2}"}}
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        });

        let mapped =
            openai_chat_completions_response_to_anthropic_message(&response).expect("map response");
        let blocks = mapped
            .get("content")
            .and_then(Value::as_array)
            .expect("content blocks");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].get("id").and_then(Value::as_str), Some("call_0"));
        assert_eq!(blocks[1].get("id").and_then(Value::as_str), Some("call_1"));
    }
}
