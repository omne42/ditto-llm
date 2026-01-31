
pub fn generate_response_to_chat_completions(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut content = String::new();
    let mut tool_calls = Vec::<Value>::new();
    for (idx, part) in response.content.iter().enumerate() {
        match part {
            ContentPart::Text { text } => content.push_str(text),
            ContentPart::ToolCall {
                id: call_id,
                name,
                arguments,
            } => {
                let call_id = call_id.trim();
                let call_id = if call_id.is_empty() {
                    format!("call_{idx}")
                } else {
                    call_id.to_string()
                };
                let arguments = arguments.to_string();
                tool_calls.push(serde_json::json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
            _ => {}
        }
    }

    let mut message = Map::<String, Value>::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    if !content.is_empty() {
        message.insert("content".to_string(), Value::String(content));
    } else {
        message.insert("content".to_string(), Value::Null);
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    let finish_reason = finish_reason_to_chat_finish_reason(response.finish_reason);

    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("message".to_string(), Value::Object(message));
    if let Some(finish_reason) = finish_reason {
        choice.insert(
            "finish_reason".to_string(),
            Value::String(finish_reason.to_string()),
        );
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert(
        "choices".to_string(),
        Value::Array(vec![Value::Object(choice)]),
    );

    if let Some(usage) = usage_to_chat_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }

    Value::Object(out)
}

pub fn generate_response_to_completions(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut text = String::new();
    for part in &response.content {
        if let ContentPart::Text { text: delta } = part {
            text.push_str(delta);
        }
    }

    let finish_reason = finish_reason_to_chat_finish_reason(response.finish_reason);

    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("text".to_string(), Value::String(text));
    choice.insert("logprobs".to_string(), Value::Null);
    if let Some(finish_reason) = finish_reason {
        choice.insert(
            "finish_reason".to_string(),
            Value::String(finish_reason.to_string()),
        );
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("text_completion".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert(
        "choices".to_string(),
        Value::Array(vec![Value::Object(choice)]),
    );
    if let Some(usage) = usage_to_chat_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }

    Value::Object(out)
}

pub fn generate_response_to_responses(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut output_text = String::new();
    let mut output_items = Vec::<Value>::new();
    let mut tool_calls = Vec::<Value>::new();

    for (idx, part) in response.content.iter().enumerate() {
        match part {
            ContentPart::Text { text } => output_text.push_str(text),
            ContentPart::ToolCall {
                id: call_id,
                name,
                arguments,
            } => {
                let call_id = call_id.trim();
                let call_id = if call_id.is_empty() {
                    format!("call_{idx}")
                } else {
                    call_id.to_string()
                };
                tool_calls.push(serde_json::json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments.to_string(),
                }));
            }
            _ => {}
        }
    }

    if !output_text.is_empty() {
        output_items.push(serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type":"output_text", "text": output_text}],
        }));
    }
    output_items.extend(tool_calls);

    let (status, incomplete_details) = finish_reason_to_responses_status(response.finish_reason);

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert("object".to_string(), Value::String("response".to_string()));
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("status".to_string(), Value::String(status.to_string()));
    if let Some(details) = incomplete_details {
        out.insert("incomplete_details".to_string(), details);
    }
    out.insert("output".to_string(), Value::Array(output_items));
    out.insert("output_text".to_string(), Value::String(output_text));
    if let Some(usage) = usage_to_responses_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }
    Value::Object(out)
}

pub fn stream_to_chat_completions_sse(
    stream: StreamResult,
    fallback_id: String,
    model: String,
    created: u64,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct State {
        response_id: String,
        tool_call_index: HashMap<String, usize>,
        finish_reason: Option<FinishReason>,
        usage: Option<Usage>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| {
            let model = model.clone();
            async move {
                loop {
                    if let Some(item) = buffer.pop_front() {
                        return Some((item, (inner, buffer, state, done)));
                    }
                    if done {
                        return None;
                    }

                    match inner.next().await {
                        Some(Ok(chunk)) => {
                            match chunk {
                                crate::types::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::types::StreamChunk::Warnings { .. } => {}
                                crate::types::StreamChunk::TextDelta { text } => {
                                    if !text.is_empty() {
                                        buffer.push_back(Ok(chat_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            serde_json::json!({"content": text}),
                                            None,
                                            None,
                                        )));
                                    }
                                }
                                crate::types::StreamChunk::ToolCallStart { id, name } => {
                                    let idx = if let Some(idx) =
                                        state.tool_call_index.get(&id).copied()
                                    {
                                        idx
                                    } else {
                                        let idx = state.tool_call_index.len();
                                        state.tool_call_index.insert(id.clone(), idx);
                                        idx
                                    };
                                    buffer.push_back(Ok(chat_chunk_bytes(
                                        &state.response_id,
                                        &model,
                                        created,
                                        serde_json::json!({
                                            "tool_calls": [{
                                                "index": idx,
                                                "id": id,
                                                "type": "function",
                                                "function": { "name": name }
                                            }]
                                        }),
                                        None,
                                        None,
                                    )));
                                }
                                crate::types::StreamChunk::ToolCallDelta {
                                    id,
                                    arguments_delta,
                                } => {
                                    let idx = if let Some(idx) =
                                        state.tool_call_index.get(&id).copied()
                                    {
                                        idx
                                    } else {
                                        let idx = state.tool_call_index.len();
                                        state.tool_call_index.insert(id.clone(), idx);
                                        idx
                                    };
                                    if !arguments_delta.is_empty() {
                                        buffer.push_back(Ok(chat_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            serde_json::json!({
                                                "tool_calls": [{
                                                    "index": idx,
                                                    "id": id,
                                                    "type": "function",
                                                    "function": { "arguments": arguments_delta }
                                                }]
                                            }),
                                            None,
                                            None,
                                        )));
                                    }
                                }
                                crate::types::StreamChunk::ReasoningDelta { .. } => {}
                                crate::types::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::types::StreamChunk::Usage(usage) => {
                                    state.usage = Some(usage);
                                }
                            }
                            continue;
                        }
                        Some(Err(err)) => {
                            buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            done = true;
                            continue;
                        }
                        None => {
                            let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                            buffer.push_back(Ok(chat_chunk_bytes(
                                &state.response_id,
                                &model,
                                created,
                                serde_json::json!({}),
                                Some(finish_reason),
                                None,
                            )));
                            if let Some(usage) = state.usage.as_ref().and_then(usage_to_chat_usage)
                            {
                                buffer.push_back(Ok(chat_usage_chunk_bytes(
                                    &state.response_id,
                                    &model,
                                    created,
                                    usage,
                                )));
                            }
                            buffer.push_back(Ok(Bytes::from("data: [DONE]\n\n")));
                            done = true;
                            continue;
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

pub fn stream_to_completions_sse(
    stream: StreamResult,
    fallback_id: String,
    model: String,
    created: u64,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct State {
        response_id: String,
        finish_reason: Option<FinishReason>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| {
            let model = model.clone();
            async move {
                loop {
                    if let Some(item) = buffer.pop_front() {
                        return Some((item, (inner, buffer, state, done)));
                    }
                    if done {
                        return None;
                    }

                    match inner.next().await {
                        Some(Ok(chunk)) => {
                            match chunk {
                                crate::types::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::types::StreamChunk::Warnings { .. } => {}
                                crate::types::StreamChunk::TextDelta { text } => {
                                    if !text.is_empty() {
                                        buffer.push_back(Ok(completion_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            &text,
                                            None,
                                        )));
                                    }
                                }
                                crate::types::StreamChunk::ToolCallStart { .. } => {}
                                crate::types::StreamChunk::ToolCallDelta { .. } => {}
                                crate::types::StreamChunk::ReasoningDelta { .. } => {}
                                crate::types::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::types::StreamChunk::Usage(_) => {}
                            }
                            continue;
                        }
                        Some(Err(err)) => {
                            buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            done = true;
                            continue;
                        }
                        None => {
                            let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                            buffer.push_back(Ok(completion_chunk_bytes(
                                &state.response_id,
                                &model,
                                created,
                                "",
                                Some(finish_reason),
                            )));
                            buffer.push_back(Ok(Bytes::from("data: [DONE]\n\n")));
                            done = true;
                            continue;
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

pub fn stream_to_responses_sse(
    stream: StreamResult,
    fallback_id: String,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct ToolCallState {
        id: String,
        name: String,
        pending_arguments: String,
    }

    #[derive(Default)]
    struct State {
        response_id: String,
        created_sent: bool,
        tool_call_index: HashMap<String, usize>,
        tool_calls: Vec<ToolCallState>,
        finish_reason: Option<FinishReason>,
        usage: Option<Usage>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((item, (inner, buffer, state, done)));
                }
                if done {
                    return None;
                }

                match inner.next().await {
                    Some(Ok(chunk)) => {
                        if let crate::types::StreamChunk::ResponseId { id } = &chunk {
                            let id = id.trim();
                            if !id.is_empty() {
                                state.response_id = id.to_string();
                            }
                        }

                        if !state.created_sent {
                            let response_id = state.response_id.clone();
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.created",
                                "response": { "id": response_id }
                            }))));
                            state.created_sent = true;
                        }

                        match chunk {
                            crate::types::StreamChunk::Warnings { .. } => {}
                            crate::types::StreamChunk::ResponseId { .. } => {}
                            crate::types::StreamChunk::TextDelta { text } => {
                                if !text.is_empty() {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.output_text.delta",
                                        "delta": text,
                                    }))));
                                }
                            }
                            crate::types::StreamChunk::ToolCallStart { id, name } => {
                                let idx = state
                                    .tool_call_index
                                    .entry(id.clone())
                                    .or_insert_with(|| state.tool_calls.len())
                                    .to_owned();
                                if state.tool_calls.len() <= idx {
                                    state
                                        .tool_calls
                                        .resize_with(idx.saturating_add(1), ToolCallState::default);
                                }
                                let slot = &mut state.tool_calls[idx];
                                slot.id = id;
                                slot.name = name;
                            }
                            crate::types::StreamChunk::ToolCallDelta {
                                id,
                                arguments_delta,
                            } => {
                                let idx = state
                                    .tool_call_index
                                    .entry(id.clone())
                                    .or_insert_with(|| state.tool_calls.len())
                                    .to_owned();
                                if state.tool_calls.len() <= idx {
                                    state
                                        .tool_calls
                                        .resize_with(idx.saturating_add(1), ToolCallState::default);
                                }
                                let slot = &mut state.tool_calls[idx];
                                if slot.id.is_empty() {
                                    slot.id = id;
                                }
                                slot.pending_arguments.push_str(&arguments_delta);
                            }
                            crate::types::StreamChunk::ReasoningDelta { .. } => {}
                            crate::types::StreamChunk::FinishReason(reason) => {
                                state.finish_reason = Some(reason);
                            }
                            crate::types::StreamChunk::Usage(usage) => {
                                state.usage = Some(usage);
                            }
                        }
                        continue;
                    }
                    Some(Err(err)) => {
                        buffer.push_back(Err(std::io::Error::other(err.to_string())));
                        done = true;
                        continue;
                    }
                    None => {
                        if !state.created_sent {
                            let response_id = state.response_id.clone();
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.created",
                                "response": { "id": response_id }
                            }))));
                            state.created_sent = true;
                        }

                        for (idx, slot) in state.tool_calls.iter().enumerate() {
                            let call_id = slot.id.trim();
                            let call_id = if call_id.is_empty() {
                                format!("call_{idx}")
                            } else {
                                call_id.to_string()
                            };
                            let name = slot.name.trim();
                            let name = if name.is_empty() {
                                "unknown".to_string()
                            } else {
                                name.to_string()
                            };
                            let args = slot.pending_arguments.trim();
                            if args.is_empty() && name == "unknown" {
                                continue;
                            }
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.output_item.done",
                                "item": {
                                    "type": "function_call",
                                    "call_id": call_id,
                                    "name": name,
                                    "arguments": if args.is_empty() { "{}" } else { args },
                                }
                            }))));
                        }

                        let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                        let (status, incomplete_details) =
                            finish_reason_to_responses_status(finish_reason);

                        let mut response = Map::<String, Value>::new();
                        response.insert("id".to_string(), Value::String(state.response_id.clone()));
                        response.insert("status".to_string(), Value::String(status.to_string()));
                        if let Some(incomplete_details) = incomplete_details {
                            response.insert("incomplete_details".to_string(), incomplete_details);
                        }
                        if let Some(usage) = state.usage.as_ref().and_then(usage_to_responses_usage)
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

                        done = true;
                        continue;
                    }
                }
            }
        },
    )
    .boxed()
}

pub fn provider_response_id(response: &GenerateResponse, fallback: &str) -> String {
    response
        .provider_metadata
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| id.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn provider_response_id_from_chunk(chunk: &crate::types::StreamChunk) -> Option<String> {
    match chunk {
        crate::types::StreamChunk::ResponseId { id } => {
            let id = id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }
        }
        _ => None,
    }
}

pub fn map_provider_error_to_openai(
    err: crate::DittoError,
) -> (StatusCode, &'static str, Option<&'static str>, String) {
    match err {
        crate::DittoError::Api { status, body } => {
            let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, "api_error", Some("provider_error"), body)
        }
        other => (
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("provider_error"),
            other.to_string(),
        ),
    }
}

fn parse_openai_chat_message(message: &Value) -> ParseResult<Message> {
    let obj = message
        .as_object()
        .ok_or_else(|| "chat message must be an object".to_string())?;

    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat message missing role".to_string())?;

    let role = match role {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        other => return Err(format!("unsupported role: {other}")),
    };

    if role == Role::Tool {
        let tool_call_id = obj
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "tool message missing tool_call_id".to_string())?;
        let content = obj
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Ok(Message::tool_result(tool_call_id, content));
    }

    let mut parts = Vec::<ContentPart>::new();
    if let Some(content) = obj.get("content") {
        parts.extend(parse_openai_content_parts(content));
    }

    if role == Role::Assistant {
        if let Some(tool_calls) = obj.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                if let Some(part) = parse_openai_tool_call(call) {
                    parts.push(part);
                }
            }
        } else if let Some(function_call) = obj.get("function_call").and_then(Value::as_object) {
            if let Some(part) = parse_openai_function_call(function_call) {
                parts.push(part);
            }
        }
    }

    Ok(Message {
        role,
        content: parts,
    })
}

fn parse_openai_content_parts(value: &Value) -> Vec<ContentPart> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentPart::Text {
                    text: text.to_string(),
                }]
            }
        }
        Value::Array(items) => {
            let mut out = Vec::<ContentPart>::new();
            for item in items {
                match item {
                    Value::String(text) => {
                        if !text.is_empty() {
                            out.push(ContentPart::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                    Value::Object(obj) => {
                        if let Some(text) = obj.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                out.push(ContentPart::Text {
                                    text: text.to_string(),
                                });
                                continue;
                            }
                        }

                        let ty = obj.get("type").and_then(Value::as_str).unwrap_or_default();
                        match ty {
                            "text" | "input_text" | "output_text" => {
                                if let Some(text) = obj.get("text").and_then(Value::as_str) {
                                    if !text.is_empty() {
                                        out.push(ContentPart::Text {
                                            text: text.to_string(),
                                        });
                                    }
                                }
                            }
                            "image_url" => {
                                if let Some(url) = obj
                                    .get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    out.push(ContentPart::Image {
                                        source: ImageSource::Url {
                                            url: url.to_string(),
                                        },
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn parse_openai_tools(value: &Value) -> ParseResult<Vec<Tool>> {
    let items = value
        .as_array()
        .ok_or_else(|| "tools must be an array".to_string())?;

    let mut out = Vec::<Tool>::new();
    for tool in items {
        let obj = match tool.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let ty = obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if ty != "function" {
            continue;
        }

        let function = obj
            .get("function")
            .and_then(Value::as_object)
            .unwrap_or(obj);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "tool missing function.name".to_string())?;
        let description = function
            .get("description")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let parameters = function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let strict = function.get("strict").and_then(Value::as_bool);

        out.push(Tool {
            name: name.to_string(),
            description,
            parameters,
            strict,
        });
    }
    Ok(out)
}

fn parse_openai_tool_choice(value: &Value) -> ParseResult<Option<ToolChoice>> {
    match value {
        Value::String(choice) => match choice.as_str() {
            "auto" => Ok(Some(ToolChoice::Auto)),
            "none" => Ok(Some(ToolChoice::None)),
            "required" => Ok(Some(ToolChoice::Required)),
            other => Err(format!("unsupported tool_choice: {other}")),
        },
        Value::Object(obj) => {
            let name = obj
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .or_else(|| obj.get("name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "tool_choice missing function.name".to_string())?;
            Ok(Some(ToolChoice::Tool {
                name: name.to_string(),
            }))
        }
        _ => Ok(None),
    }
}

fn parse_openai_tool_call(value: &Value) -> Option<ContentPart> {
    let obj = value.as_object()?;
    let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
    let function = obj.get("function").and_then(Value::as_object)?;
    let name = function.get("name").and_then(Value::as_str)?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let parsed_arguments = serde_json::from_str::<Value>(arguments)
        .unwrap_or_else(|_| Value::String(arguments.into()));

    Some(ContentPart::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: parsed_arguments,
    })
}

fn parse_openai_function_call(obj: &Map<String, Value>) -> Option<ContentPart> {
    let name = obj.get("name").and_then(Value::as_str)?;
    let arguments = obj.get("arguments").and_then(Value::as_str).unwrap_or("{}");
    let parsed_arguments = serde_json::from_str::<Value>(arguments)
        .unwrap_or_else(|_| Value::String(arguments.into()));
    Some(ContentPart::ToolCall {
        id: String::new(),
        name: name.to_string(),
        arguments: parsed_arguments,
    })
}

fn parse_stop_sequences(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(stop) => {
            let stop = stop.trim();
            if stop.is_empty() {
                None
            } else {
                Some(vec![stop.to_string()])
            }
        }
        Value::Array(values) => {
            let mut out = Vec::<String>::new();
            for value in values {
                if let Some(stop) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    out.push(stop.to_string());
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

fn parse_provider_options_from_openai_request(obj: &Map<String, Value>) -> ProviderOptions {
    let mut out = ProviderOptions::default();

    if let Some(reasoning) = obj.get("reasoning").and_then(Value::as_object) {
        if let Some(effort) = reasoning
            .get("effort")
            .and_then(Value::as_str)
            .and_then(parse_reasoning_effort)
        {
            out.reasoning_effort = Some(effort);
        }
    }

    if let Some(parallel) = obj.get("parallel_tool_calls").and_then(Value::as_bool) {
        out.parallel_tool_calls = Some(parallel);
    }

    if let Some(format_value) = obj.get("response_format").and_then(Value::as_object) {
        if let Some(parsed) = parse_json_schema_response_format(format_value) {
            out.response_format = Some(parsed);
        }
    }

    out
}
