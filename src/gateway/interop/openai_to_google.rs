fn openai_finish_reason_to_google(reason: Option<&str>) -> &'static str {
    match reason {
        Some("stop") | None => "STOP",
        Some("length") => "MAX_TOKENS",
        Some("content_filter") => "SAFETY",
        Some("tool_calls") | Some("function_call") => "STOP",
        Some(_) => "STOP",
    }
}

pub(crate) fn openai_chat_completions_response_to_google_generate_content(
    response: &Value,
) -> Result<Value, String> {
    let obj = response
        .as_object()
        .ok_or_else(|| "openai response must be a JSON object".to_string())?;

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
    let finish_reason = openai_finish_reason_to_google(finish_reason);

    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "openai response missing choices[0].message".to_string())?;

    let mut parts = Vec::<Value>::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            parts.push(serde_json::json!({ "text": text }));
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let Some(call) = call.as_object() else {
                continue;
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
            let args = serde_json::from_str::<Value>(args_raw).unwrap_or(Value::Object(Map::new()));
            parts.push(serde_json::json!({
                "functionCall": { "name": name, "args": args }
            }));
        }
    }

    let usage = obj.get("usage").map(parse_openai_usage).unwrap_or_default();
    let usage_metadata = serde_json::json!({
        "promptTokenCount": usage.prompt_tokens.unwrap_or(0),
        "candidatesTokenCount": usage.completion_tokens.unwrap_or(0),
        "totalTokenCount": usage.total_tokens.unwrap_or(0),
    });

    Ok(serde_json::json!({
        "candidates": [{
            "index": 0,
            "finishReason": finish_reason,
            "content": { "role": "model", "parts": parts },
        }],
        "usageMetadata": usage_metadata,
    }))
}

pub(crate) fn openai_chat_completions_response_to_cloudcode_generate_content(
    response: &Value,
) -> Result<Value, String> {
    let openai_obj = response
        .as_object()
        .ok_or_else(|| "openai response must be a JSON object".to_string())?;
    let response_id = openai_obj
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl_unknown")
        .to_string();
    let model = openai_obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let inner = openai_chat_completions_response_to_google_generate_content(response)?;
    let mut inner_obj = inner
        .as_object()
        .cloned()
        .unwrap_or_else(Map::<String, Value>::new);
    inner_obj.insert("responseId".to_string(), Value::String(response_id));
    if !model.trim().is_empty() {
        inner_obj.insert("modelVersion".to_string(), Value::String(model));
    }

    Ok(serde_json::json!({
        "response": Value::Object(inner_obj),
    }))
}

fn anthropic_sse_bytes(event: &str, payload: Value) -> Bytes {
    let json = payload.to_string();
    Bytes::from(format!("event: {event}\ndata: {json}\n\n"))
}

fn google_sse_bytes(payload: Value) -> Bytes {
    let json = payload.to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

#[derive(Debug, Default)]
struct ToolCallBlock {
    id: Option<String>,
    name: Option<String>,
    pending_arguments: String,
    started: bool,
}

#[derive(Debug)]
pub(crate) struct AnthropicSseEncoder {
    message_id: String,
    model: String,
    started: bool,
    text_started: bool,
    tool_calls: BTreeMap<usize, ToolCallBlock>,
    finish_reason: Option<String>,
    usage: OpenAiUsage,
}

impl AnthropicSseEncoder {
    pub(crate) fn new(fallback_id: String) -> Self {
        Self {
            message_id: fallback_id,
            model: String::new(),
            started: false,
            text_started: false,
            tool_calls: BTreeMap::new(),
            finish_reason: None,
            usage: OpenAiUsage::default(),
        }
    }

    fn ensure_message_start(&mut self, buffer: &mut Vec<Bytes>) {
        if self.started {
            return;
        }
        let mut message = Map::<String, Value>::new();
        message.insert("id".to_string(), Value::String(self.message_id.clone()));
        message.insert("type".to_string(), Value::String("message".to_string()));
        message.insert("role".to_string(), Value::String("assistant".to_string()));
        if !self.model.trim().is_empty() {
            message.insert("model".to_string(), Value::String(self.model.clone()));
        }
        message.insert("content".to_string(), Value::Array(Vec::new()));
        message.insert("stop_reason".to_string(), Value::Null);
        message.insert("stop_sequence".to_string(), Value::Null);
        buffer.push(anthropic_sse_bytes(
            "message_start",
            serde_json::json!({ "type": "message_start", "message": Value::Object(message) }),
        ));
        self.started = true;
    }

    fn ensure_text_start(&mut self, buffer: &mut Vec<Bytes>) {
        if self.text_started {
            return;
        }
        self.ensure_message_start(buffer);
        buffer.push(anthropic_sse_bytes(
            "content_block_start",
            serde_json::json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }),
        ));
        self.text_started = true;
    }

    fn ensure_tool_start(&mut self, openai_index: usize, buffer: &mut Vec<Bytes>) {
        let Some((id, name, pending_arguments)) =
            self.tool_calls.get_mut(&openai_index).and_then(|entry| {
                if entry.started {
                    return None;
                }
                let id = entry.id.as_deref()?.trim();
                let name = entry.name.as_deref()?.trim();
                if id.is_empty() || name.is_empty() {
                    return None;
                }
                entry.started = true;
                Some((
                    id.to_string(),
                    name.to_string(),
                    std::mem::take(&mut entry.pending_arguments),
                ))
            })
        else {
            return;
        };

        self.ensure_message_start(buffer);
        let block_index = openai_index.saturating_add(1);
        buffer.push(anthropic_sse_bytes(
            "content_block_start",
            serde_json::json!({
                "type": "content_block_start",
                "index": block_index,
                "content_block": {
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": {},
                }
            }),
        ));
        if !pending_arguments.is_empty() {
            buffer.push(anthropic_sse_bytes(
                "content_block_delta",
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": { "type": "input_json_delta", "partial_json": pending_arguments }
                }),
            ));
        }
    }

    pub(crate) fn push_openai_chunk(&mut self, data: &str) -> Result<Vec<Bytes>, String> {
        let value: Value =
            serde_json::from_str(data).map_err(|err| format!("invalid SSE JSON: {err}"))?;

        if let Some(id) = value.get("id").and_then(Value::as_str).map(str::trim) {
            if !id.is_empty() {
                self.message_id = id.to_string();
            }
        }
        if let Some(model) = value.get("model").and_then(Value::as_str).map(str::trim) {
            if !model.is_empty() {
                self.model = model.to_string();
            }
        }

        if let Some(usage) = value.get("usage") {
            self.usage = parse_openai_usage(usage);
        }

        let mut out = Vec::<Bytes>::new();
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(Value::as_object)
        else {
            return Ok(out);
        };

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            let reason = reason.trim();
            if !reason.is_empty() {
                self.finish_reason = Some(reason.to_string());
            }
        }

        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(Map::<String, Value>::new);

        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.ensure_text_start(&mut out);
                out.push(anthropic_sse_bytes(
                    "content_block_delta",
                    serde_json::json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": { "type": "text_delta", "text": text }
                    }),
                ));
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let Some(obj) = tool_call.as_object() else {
                    continue;
                };
                let index = obj.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let entry = self.tool_calls.entry(index).or_default();
                if let Some(id) = obj.get("id").and_then(Value::as_str).map(str::trim) {
                    if !id.is_empty() {
                        entry.id = Some(id.to_string());
                    }
                }
                if let Some(function) = obj.get("function").and_then(Value::as_object) {
                    if let Some(name) = function.get("name").and_then(Value::as_str).map(str::trim)
                    {
                        if !name.is_empty() {
                            entry.name = Some(name.to_string());
                        }
                    }
                    if let Some(args) = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        if entry.started {
                            let block_index = index.saturating_add(1);
                            out.push(anthropic_sse_bytes(
                                "content_block_delta",
                                serde_json::json!({
                                    "type": "content_block_delta",
                                    "index": block_index,
                                    "delta": { "type": "input_json_delta", "partial_json": args }
                                }),
                            ));
                        } else {
                            entry.pending_arguments.push_str(args);
                        }
                    }
                }

                self.ensure_tool_start(index, &mut out);
            }
        }

        Ok(out)
    }

    pub(crate) fn finish(mut self) -> Vec<Bytes> {
        let mut out = Vec::<Bytes>::new();
        self.ensure_message_start(&mut out);

        if self.text_started {
            out.push(anthropic_sse_bytes(
                "content_block_stop",
                serde_json::json!({ "type": "content_block_stop", "index": 0 }),
            ));
        }

        for (idx, block) in self.tool_calls.into_iter() {
            if !block.started {
                continue;
            }
            let block_index = idx.saturating_add(1);
            out.push(anthropic_sse_bytes(
                "content_block_stop",
                serde_json::json!({ "type": "content_block_stop", "index": block_index }),
            ));
        }

        let stop_reason =
            openai_finish_reason_to_anthropic(self.finish_reason.as_deref()).unwrap_or("end_turn");
        out.push(anthropic_sse_bytes(
            "message_delta",
            serde_json::json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": {
                    "input_tokens": self.usage.prompt_tokens.unwrap_or(0),
                    "output_tokens": self.usage.completion_tokens.unwrap_or(0),
                }
            }),
        ));
        out.push(anthropic_sse_bytes(
            "message_stop",
            serde_json::json!({ "type": "message_stop" }),
        ));
        out
    }
}

#[derive(Debug)]
pub(crate) struct GoogleSseEncoder {
    response_id: String,
    model: String,
    finish_reason: Option<String>,
    usage: OpenAiUsage,
    wrap_cloudcode: bool,
}

impl GoogleSseEncoder {
    pub(crate) fn new(fallback_id: String, wrap_cloudcode: bool) -> Self {
        Self {
            response_id: fallback_id,
            model: String::new(),
            finish_reason: None,
            usage: OpenAiUsage::default(),
            wrap_cloudcode,
        }
    }

    fn wrap_if_needed(&self, payload: Value) -> Bytes {
        if self.wrap_cloudcode {
            google_sse_bytes(serde_json::json!({ "response": payload }))
        } else {
            google_sse_bytes(payload)
        }
    }

    pub(crate) fn push_openai_chunk(&mut self, data: &str) -> Result<Vec<Bytes>, String> {
        let value: Value =
            serde_json::from_str(data).map_err(|err| format!("invalid SSE JSON: {err}"))?;

        if let Some(id) = value.get("id").and_then(Value::as_str).map(str::trim) {
            if !id.is_empty() {
                self.response_id = id.to_string();
            }
        }
        if let Some(model) = value.get("model").and_then(Value::as_str).map(str::trim) {
            if !model.is_empty() {
                self.model = model.to_string();
            }
        }
        if let Some(usage) = value.get("usage") {
            self.usage = parse_openai_usage(usage);
        }

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(Value::as_object)
        else {
            return Ok(Vec::new());
        };

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            let reason = reason.trim();
            if !reason.is_empty() {
                self.finish_reason = Some(reason.to_string());
            }
        }

        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(Map::<String, Value>::new);

        let Some(text_delta) = delta.get("content").and_then(Value::as_str) else {
            return Ok(Vec::new());
        };
        if text_delta.is_empty() {
            return Ok(Vec::new());
        }

        let chunk = serde_json::json!({
            "candidates": [{
                "index": 0,
                "content": { "role": "model", "parts": [{ "text": text_delta }] },
            }]
        });
        Ok(vec![self.wrap_if_needed(chunk)])
    }

    pub(crate) fn finish(self) -> Bytes {
        let finish_reason = openai_finish_reason_to_google(self.finish_reason.as_deref());
        let usage_metadata = serde_json::json!({
            "promptTokenCount": self.usage.prompt_tokens.unwrap_or(0),
            "candidatesTokenCount": self.usage.completion_tokens.unwrap_or(0),
            "totalTokenCount": self.usage.total_tokens.unwrap_or(0),
        });

        let mut out = Map::<String, Value>::new();
        if self.wrap_cloudcode {
            out.insert("responseId".to_string(), Value::String(self.response_id));
            if !self.model.trim().is_empty() {
                out.insert("modelVersion".to_string(), Value::String(self.model));
            }
        }
        out.insert(
            "candidates".to_string(),
            Value::Array(vec![serde_json::json!({
                "index": 0,
                "finishReason": finish_reason,
                "content": { "role": "model", "parts": [] },
            })]),
        );
        out.insert("usageMetadata".to_string(), usage_metadata);

        if self.wrap_cloudcode {
            google_sse_bytes(serde_json::json!({ "response": Value::Object(out) }))
        } else {
            google_sse_bytes(Value::Object(out))
        }
    }
}
