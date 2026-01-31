
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

fn usage_to_chat_usage(usage: &Usage) -> Option<Value> {
    let prompt = usage.input_tokens?;
    let completion = usage.output_tokens?;
    let total = usage
        .total_tokens
        .or(Some(prompt.saturating_add(completion)))?;
    Some(serde_json::json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    }))
}

fn usage_to_responses_usage(usage: &Usage) -> Option<Value> {
    let mut out = Map::<String, Value>::new();
    if let Some(input_tokens) = usage.input_tokens {
        out.insert(
            "input_tokens".to_string(),
            Value::Number((input_tokens as i64).into()),
        );
    }
    if let Some(output_tokens) = usage.output_tokens {
        out.insert(
            "output_tokens".to_string(),
            Value::Number((output_tokens as i64).into()),
        );
    }
    if let Some(total_tokens) = usage.total_tokens.or_else(|| {
        usage
            .input_tokens
            .zip(usage.output_tokens)
            .map(|(i, o)| i.saturating_add(o))
    }) {
        out.insert(
            "total_tokens".to_string(),
            Value::Number((total_tokens as i64).into()),
        );
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn finish_reason_to_chat_finish_reason(reason: FinishReason) -> Option<&'static str> {
    match reason {
        FinishReason::Stop => Some("stop"),
        FinishReason::Length => Some("length"),
        FinishReason::ToolCalls => Some("tool_calls"),
        FinishReason::ContentFilter => Some("content_filter"),
        FinishReason::Error => Some("error"),
        FinishReason::Unknown => None,
    }
}

fn finish_reason_to_responses_status(reason: FinishReason) -> (&'static str, Option<Value>) {
    match reason {
        FinishReason::Length => (
            "incomplete",
            Some(serde_json::json!({ "reason": "max_output_tokens" })),
        ),
        FinishReason::ContentFilter => (
            "incomplete",
            Some(serde_json::json!({ "reason": "content_filter" })),
        ),
        FinishReason::Error => ("failed", None),
        _ => ("completed", None),
    }
}

fn completion_chunk_bytes(
    id: &str,
    model: &str,
    created: u64,
    text: &str,
    finish_reason: Option<FinishReason>,
) -> Bytes {
    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("text".to_string(), Value::String(text.to_string()));
    choice.insert("logprobs".to_string(), Value::Null);
    if let Some(finish_reason) = finish_reason {
        if let Some(mapped) = finish_reason_to_chat_finish_reason(finish_reason) {
            choice.insert(
                "finish_reason".to_string(),
                Value::String(mapped.to_string()),
            );
        } else {
            choice.insert("finish_reason".to_string(), Value::Null);
        }
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

    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn chat_chunk_bytes(
    id: &str,
    model: &str,
    created: u64,
    delta: Value,
    finish_reason: Option<FinishReason>,
    usage: Option<Value>,
) -> Bytes {
    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("delta".to_string(), delta);
    if let Some(finish_reason) = finish_reason {
        if let Some(mapped) = finish_reason_to_chat_finish_reason(finish_reason) {
            choice.insert(
                "finish_reason".to_string(),
                Value::String(mapped.to_string()),
            );
        } else {
            choice.insert("finish_reason".to_string(), Value::Null);
        }
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion.chunk".to_string()),
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
    if let Some(usage) = usage {
        out.insert("usage".to_string(), usage);
    }

    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn chat_usage_chunk_bytes(id: &str, model: &str, created: u64, usage: Value) -> Bytes {
    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion.chunk".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("choices".to_string(), Value::Array(Vec::new()));
    out.insert("usage".to_string(), usage);
    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn sse_event_bytes(value: Value) -> Bytes {
    let json = value.to_string();
    Bytes::from(format!("data: {json}\n\n"))
}
