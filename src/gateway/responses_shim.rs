use std::collections::VecDeque;

use axum::http::{Method, StatusCode};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize, Default)]
struct ChatCompletionsResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCall>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ChatFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ChatToolCall {
    #[serde(default)]
    id: String,
    #[serde(default)]
    function: ChatToolFunction,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ChatToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceChunk>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatChoiceChunk {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCallDelta>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatFunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatToolFunctionDelta>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Default)]
struct ToolCallState {
    id: Option<String>,
    name: Option<String>,
    pending_arguments: String,
}

#[derive(Debug, Default)]
struct StreamState {
    response_id: Option<String>,
    created_sent: bool,
    tool_calls: Vec<ToolCallState>,
    finish_reason: Option<String>,
    usage: Option<Value>,
}

const MAX_STREAM_TOOL_CALL_SLOTS: usize = 256;

pub fn is_responses_create_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses" || path == "/v1/responses/"
}

pub fn should_attempt_responses_shim(
    method: &Method,
    path_and_query: &str,
    upstream_status: StatusCode,
) -> bool {
    if method != Method::POST {
        return false;
    }
    if !is_responses_create_path(path_and_query) {
        return false;
    }
    matches!(
        upstream_status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
    )
}

pub fn responses_request_to_chat_completions(request: &Value) -> Option<Value> {
    let obj = request.as_object()?;

    let mut out = Map::<String, Value>::new();

    let model = obj.get("model")?.clone();
    out.insert("model".to_string(), model);

    if let Some(temperature) = obj.get("temperature") {
        out.insert("temperature".to_string(), temperature.clone());
    }
    if let Some(top_p) = obj.get("top_p") {
        out.insert("top_p".to_string(), top_p.clone());
    }
    if let Some(max_output_tokens) = obj.get("max_output_tokens") {
        out.insert("max_tokens".to_string(), max_output_tokens.clone());
    }
    if let Some(service_tier) = obj.get("service_tier") {
        out.insert("service_tier".to_string(), service_tier.clone());
    }

    if let Some(tools) = obj.get("tools") {
        out.insert("tools".to_string(), tools.clone());
    }
    if let Some(tool_choice) = obj.get("tool_choice") {
        out.insert("tool_choice".to_string(), tool_choice.clone());
    }
    if let Some(response_format) = obj.get("response_format") {
        out.insert("response_format".to_string(), response_format.clone());
    }

    let stream = obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    if stream {
        out.insert("stream".to_string(), Value::Bool(true));
        let mut stream_options = obj
            .get("stream_options")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        stream_options
            .entry("include_usage".to_string())
            .or_insert(Value::Bool(true));
        out.insert("stream_options".to_string(), Value::Object(stream_options));
    }

    let mut messages = Vec::<Value>::new();
    if let Some(instructions) = obj
        .get("instructions")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        messages.push(serde_json::json!({
            "role": "system",
            "content": instructions,
        }));
    }

    if let Some(input) = obj.get("input") {
        append_messages_from_responses_input(&mut messages, input);
    } else if let Some(existing) = obj.get("messages") {
        if let Some(arr) = existing.as_array() {
            messages.extend(arr.iter().cloned());
        }
    }

    if messages.is_empty() {
        return None;
    }

    out.insert("messages".to_string(), Value::Array(messages));
    Some(Value::Object(out))
}

pub fn chat_completions_response_to_responses(chat_response: &Value) -> Option<Value> {
    let parsed: ChatCompletionsResponse = serde_json::from_value(chat_response.clone()).ok()?;
    if parsed.id.trim().is_empty() {
        return None;
    }

    let ChatCompletionsResponse {
        id,
        model,
        choices,
        usage,
    } = parsed;

    let ChatChoice {
        message,
        finish_reason,
    } = choices.into_iter().next().unwrap_or_default();
    let finish = finish_reason.as_deref().unwrap_or("stop");
    let (status, incomplete_details) = map_finish_reason_to_status(finish);

    let ChatMessage {
        content,
        tool_calls,
        function_call,
    } = message;
    let content = content.unwrap_or_default();
    let output_text = content.clone();

    let mut output_items = Vec::<Value>::new();
    if !content.is_empty() {
        output_items.push(serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type":"output_text", "text": content}],
        }));
    }

    let mut tool_calls = tool_calls.unwrap_or_default();
    if tool_calls.is_empty() {
        if let Some(call) = function_call {
            if !call.name.trim().is_empty() {
                tool_calls.push(ChatToolCall {
                    id: String::new(),
                    function: ChatToolFunction {
                        name: call.name,
                        arguments: call.arguments,
                    },
                });
            }
        }
    }

    for (idx, call) in tool_calls.into_iter().enumerate() {
        let call_id = match call.id.trim() {
            "" => format!("call_{idx}"),
            value => value.to_string(),
        };
        let name = call.function.name.trim();
        if name.is_empty() {
            continue;
        }
        let arguments_raw = call.function.arguments.trim();
        let arguments = if arguments_raw.is_empty() {
            "{}".to_string()
        } else {
            arguments_raw.to_string()
        };
        output_items.push(serde_json::json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        }));
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id));
    out.insert("object".to_string(), Value::String("response".to_string()));
    out.insert("status".to_string(), Value::String(status.to_string()));
    out.insert("output".to_string(), Value::Array(output_items));
    out.insert("output_text".to_string(), Value::String(output_text));
    if let Some(incomplete_details) = incomplete_details {
        out.insert("incomplete_details".to_string(), incomplete_details);
    }
    if let Some(model) = model {
        out.insert("model".to_string(), Value::String(model));
    }
    if let Some(usage) = usage.as_ref().and_then(map_chat_usage_to_responses_usage) {
        out.insert("usage".to_string(), usage);
    }

    Some(Value::Object(out))
}

pub fn chat_completions_sse_to_responses_sse(
    data_stream: impl futures_util::Stream<Item = crate::Result<String>> + Unpin + Send + 'static,
    fallback_response_id: String,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let state = StreamState::default();
    let buffer = VecDeque::<Result<Bytes, std::io::Error>>::new();

    stream::unfold(
        (data_stream, buffer, state, false, fallback_response_id),
        |(mut data_stream, mut buffer, mut state, mut done, fallback_response_id)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((
                        item,
                        (data_stream, buffer, state, done, fallback_response_id),
                    ));
                }

                if done {
                    return None;
                }

                match data_stream.next().await {
                    Some(Ok(data)) => {
                        if let Ok(chunk) = serde_json::from_str::<ChatCompletionsChunk>(&data) {
                            if state.response_id.is_none() {
                                state.response_id = chunk
                                    .id
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|id| !id.is_empty())
                                    .map(|id| id.to_string());
                            }
                            if !state.created_sent {
                                if let Some(id) = state
                                    .response_id
                                    .as_deref()
                                    .filter(|id| !id.trim().is_empty())
                                {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.created",
                                        "response": { "id": id }
                                    }))));
                                    state.created_sent = true;
                                }
                            }

                            if let Some(usage) = chunk.usage {
                                state.usage = Some(usage);
                            }

                            for choice in chunk.choices {
                                let ChatDelta {
                                    content,
                                    tool_calls,
                                    function_call,
                                } = choice.delta;
                                if let Some(delta) = content {
                                    if !delta.is_empty() {
                                        buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                            "type": "response.output_text.delta",
                                            "delta": delta,
                                        }))));
                                    }
                                }

                                if let Some(tool_calls) = tool_calls {
                                    for delta in tool_calls {
                                        apply_tool_call_delta(&mut state, &delta);
                                    }
                                } else if let Some(function_call) = function_call {
                                    let tool_call = ChatToolCallDelta {
                                        index: 0,
                                        id: None,
                                        function: Some(ChatToolFunctionDelta {
                                            name: function_call.name,
                                            arguments: function_call.arguments,
                                        }),
                                    };
                                    apply_tool_call_delta(&mut state, &tool_call);
                                }

                                if let Some(reason) = choice.finish_reason {
                                    state.finish_reason = Some(reason);
                                }
                            }
                            continue;
                        }

                        if let Ok(value) = serde_json::from_str::<Value>(&data) {
                            if value.get("error").is_some() {
                                let response_id = state
                                    .response_id
                                    .clone()
                                    .unwrap_or_else(|| fallback_response_id.clone());
                                buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.failed",
                                        "response": { "id": response_id, "error": value.get("error").cloned().unwrap_or(Value::Null) }
                                    }))));
                                done = true;
                                continue;
                            }
                        }
                    }
                    Some(Err(err)) => {
                        buffer.push_back(Err(std::io::Error::other(err.to_string())));
                        done = true;
                        continue;
                    }
                    None => {
                        finalize_stream(&mut buffer, &mut state, fallback_response_id.as_str());
                        done = true;
                        continue;
                    }
                }
            }
        },
    )
}

fn append_messages_from_responses_input(out: &mut Vec<Value>, input: &Value) {
    match input {
        Value::String(text) => {
            if !text.trim().is_empty() {
                out.push(serde_json::json!({"role":"user","content": text}));
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(text) => {
                        if !text.trim().is_empty() {
                            out.push(serde_json::json!({"role":"user","content": text}));
                        }
                    }
                    Value::Object(obj) => {
                        let role = obj
                            .get("role")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        let Some(role) = role else {
                            continue;
                        };
                        let content = extract_message_text(obj).unwrap_or_default();
                        out.push(serde_json::json!({"role": role, "content": content}));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn extract_message_text(obj: &Map<String, Value>) -> Option<String> {
    if let Some(text) = obj.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    let parts = obj.get("content").and_then(Value::as_array)?;
    let mut out = String::new();
    for part in parts {
        match part {
            Value::String(text) => out.push_str(text),
            Value::Object(obj) => {
                if let Some(text) = obj.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

fn map_finish_reason_to_status(finish_reason: &str) -> (&'static str, Option<Value>) {
    match finish_reason {
        "length" => (
            "incomplete",
            Some(serde_json::json!({ "reason": "max_output_tokens" })),
        ),
        "content_filter" => (
            "incomplete",
            Some(serde_json::json!({ "reason": "content_filter" })),
        ),
        _ => ("completed", None),
    }
}

fn map_chat_usage_to_responses_usage(usage: &Value) -> Option<Value> {
    let obj = usage.as_object()?;
    let prompt_tokens = obj.get("prompt_tokens")?.as_u64()?;
    let completion_tokens = obj.get("completion_tokens")?.as_u64()?;
    let total_tokens = obj.get("total_tokens")?.as_u64()?;
    Some(serde_json::json!({
        "input_tokens": prompt_tokens,
        "output_tokens": completion_tokens,
        "total_tokens": total_tokens,
    }))
}

fn apply_tool_call_delta(state: &mut StreamState, delta: &ChatToolCallDelta) {
    // Bound tool call slot growth from untrusted streaming indexes.
    if delta.index >= MAX_STREAM_TOOL_CALL_SLOTS {
        return;
    }
    if state.tool_calls.len() <= delta.index {
        state
            .tool_calls
            .resize_with(delta.index + 1, ToolCallState::default);
    }
    let slot = &mut state.tool_calls[delta.index];
    if let Some(id) = delta.id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        slot.id = Some(id.to_string());
    }
    if let Some(function) = delta.function.as_ref() {
        if let Some(name) = function
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            slot.name = Some(name.to_string());
        }
        if let Some(arguments) = function.arguments.as_deref() {
            slot.pending_arguments.push_str(arguments);
        }
    }
}

fn finalize_stream(
    buffer: &mut VecDeque<Result<Bytes, std::io::Error>>,
    state: &mut StreamState,
    fallback_response_id: &str,
) {
    if state.response_id.is_none() {
        state.response_id = Some(fallback_response_id.to_string());
    }
    if !state.created_sent {
        let id = state
            .response_id
            .as_deref()
            .unwrap_or(fallback_response_id)
            .to_string();
        buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
            "type": "response.created",
            "response": { "id": id }
        }))));
        state.created_sent = true;
    }

    for (idx, slot) in state.tool_calls.iter_mut().enumerate() {
        let call_id = slot
            .id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("call_{idx}"));
        let name = slot
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("unknown");
        let args = slot.pending_arguments.trim();
        if args.is_empty() && name == "unknown" {
            continue;
        }
        let args = if args.is_empty() { "{}" } else { args };

        buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": args,
            }
        }))));
    }

    let finish = state.finish_reason.as_deref().unwrap_or("stop");
    let (status, incomplete_details) = map_finish_reason_to_status(finish);

    let mut response = Map::<String, Value>::new();
    response.insert(
        "id".to_string(),
        Value::String(
            state
                .response_id
                .clone()
                .unwrap_or_else(|| fallback_response_id.to_string()),
        ),
    );
    response.insert("status".to_string(), Value::String(status.to_string()));
    if let Some(incomplete_details) = incomplete_details {
        response.insert("incomplete_details".to_string(), incomplete_details);
    }
    if let Some(usage) = state
        .usage
        .as_ref()
        .and_then(map_chat_usage_to_responses_usage)
    {
        response.insert("usage".to_string(), usage);
    }

    let event_kind = if status == "completed" {
        "response.completed"
    } else {
        "response.incomplete"
    };
    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
        "type": event_kind,
        "response": response,
    }))));
}

fn sse_event_bytes(value: Value) -> Bytes {
    let json = serde_json::to_vec(&value).unwrap_or_else(|_| value.to_string().into_bytes());
    let mut out = BytesMut::with_capacity(6 + json.len() + 2);
    out.extend_from_slice(b"data: ");
    out.extend_from_slice(&json);
    out.extend_from_slice(b"\n\n");
    out.freeze()
}
