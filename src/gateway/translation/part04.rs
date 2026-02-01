
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

pub fn is_files_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/files" || path == "/v1/files/"
}

pub fn files_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

pub fn files_content_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    let (file_id, suffix) = rest.split_once('/')?;
    if suffix != "content" {
        return None;
    }
    let file_id = file_id.trim();
    if file_id.is_empty() {
        return None;
    }
    Some(file_id.to_string())
}

pub fn files_upload_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<FileUploadRequest> {
    let mut file: Option<MultipartPart> = None;
    let mut purpose: Option<String> = None;

    let parts = parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "file" => file = Some(part),
            "purpose" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    purpose = Some(value);
                }
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| "files request missing file".to_string())?;
    let purpose = purpose.ok_or_else(|| "files request missing purpose".to_string())?;
    let filename = file
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "file".to_string());

    Ok(FileUploadRequest {
        filename,
        bytes: file.data.to_vec(),
        purpose,
        media_type: file.content_type.clone(),
    })
}

pub fn file_upload_response_to_openai(
    file_id: &str,
    filename: String,
    purpose: String,
    bytes: usize,
    created_at: u64,
) -> Value {
    serde_json::json!({
        "id": file_id,
        "object": "file",
        "bytes": bytes,
        "created_at": created_at,
        "filename": filename,
        "purpose": purpose,
    })
}

pub fn file_to_openai(file: &crate::file::FileObject) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(file.id.clone()));
    out.insert("object".to_string(), Value::String("file".to_string()));
    out.insert("bytes".to_string(), Value::Number(file.bytes.into()));
    out.insert(
        "created_at".to_string(),
        Value::Number(file.created_at.into()),
    );
    out.insert(
        "filename".to_string(),
        Value::String(file.filename.clone()),
    );
    out.insert("purpose".to_string(), Value::String(file.purpose.clone()));
    if let Some(status) = file.status.as_deref() {
        out.insert("status".to_string(), Value::String(status.to_string()));
    }
    if let Some(details) = file.status_details.clone() {
        out.insert("status_details".to_string(), details);
    }
    Value::Object(out)
}

pub fn file_list_response_to_openai(files: &[crate::file::FileObject]) -> Value {
    Value::Object(Map::from_iter([
        ("object".to_string(), Value::String("list".to_string())),
        (
            "data".to_string(),
            Value::Array(files.iter().map(file_to_openai).collect()),
        ),
    ]))
}

pub fn file_delete_response_to_openai(response: &crate::file::FileDeleteResponse) -> Value {
    serde_json::json!({
        "id": response.id,
        "object": "file",
        "deleted": response.deleted,
    })
}

pub async fn build_file_client(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn FileClient>>> {
    let _ = (config, env);
    let provider = provider.trim();

    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Some(Arc::new(crate::OpenAI::from_config(config, env).await?)))
            }
            #[cfg(not(feature = "openai"))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
            #[cfg(feature = "openai-compatible")]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatible::from_config(config, env).await?,
                )))
            }
            #[cfg(not(feature = "openai-compatible"))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
