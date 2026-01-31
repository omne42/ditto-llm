
pub async fn build_rerank_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn RerankModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "rerank"))]
            {
                Ok(Some(Arc::new(
                    crate::CohereRerank::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "cohere", feature = "rerank")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub fn is_chat_completions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/chat/completions" || path == "/v1/chat/completions/"
}

pub fn is_completions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/completions" || path == "/v1/completions/"
}

pub fn is_models_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/models" || path == "/v1/models/"
}

pub fn models_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/models/")?;
    if rest.trim().is_empty() {
        return None;
    }
    Some(rest.to_string())
}

pub fn is_responses_create_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses" || path == "/v1/responses/"
}

pub fn is_responses_compact_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses/compact" || path == "/v1/responses/compact/"
}

pub fn is_embeddings_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/embeddings" || path == "/v1/embeddings/"
}

pub fn is_moderations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/moderations" || path == "/v1/moderations/"
}

pub fn is_images_generations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/images/generations" || path == "/v1/images/generations/"
}

pub fn is_audio_transcriptions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/transcriptions" || path == "/v1/audio/transcriptions/"
}

pub fn is_audio_translations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/translations" || path == "/v1/audio/translations/"
}

pub fn is_audio_speech_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/speech" || path == "/v1/audio/speech/"
}

pub fn is_batches_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/batches" || path == "/v1/batches/"
}

pub fn is_rerank_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/rerank" || path == "/v1/rerank/"
}

pub fn batches_cancel_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    let (batch_id, suffix) = rest.split_once('/')?;
    if batch_id.trim().is_empty() {
        return None;
    }
    if suffix == "cancel" {
        return Some(batch_id.to_string());
    }
    None
}

pub fn batches_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

pub fn collect_models_from_translation_backends(
    backends: &HashMap<String, TranslationBackend>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();

    let mut backend_names = backends.keys().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let backend = match backends.get(backend_name) {
            Some(backend) => backend,
            None => continue,
        };

        let provider = backend.provider.trim();
        let owned_by = if provider.is_empty() {
            backend_name.as_str()
        } else {
            provider
        };

        for key in backend.model_map.keys() {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            out.entry(key.to_string())
                .or_insert_with(|| owned_by.to_string());
        }

        let default_model = backend.model.model_id().trim();
        if !default_model.is_empty() {
            out.entry(format!("{owned_by}/{default_model}"))
                .or_insert_with(|| owned_by.to_string());
        }

        for value in backend.model_map.values() {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            out.entry(format!("{owned_by}/{value}"))
                .or_insert_with(|| owned_by.to_string());
        }
    }

    out
}

pub fn model_to_openai(id: &str, owned_by: &str, created: u64) -> Value {
    let id = id.trim();
    let owned_by = owned_by.trim();
    serde_json::json!({
        "id": id,
        "object": "model",
        "created": created,
        "owned_by": owned_by,
    })
}

pub fn models_list_to_openai(models: &BTreeMap<String, String>, created: u64) -> Value {
    let data = models
        .iter()
        .map(|(id, owned_by)| model_to_openai(id, owned_by, created))
        .collect::<Vec<_>>();
    serde_json::json!({
        "object": "list",
        "data": data,
    })
}

pub fn batches_create_request_to_request(request: &Value) -> ParseResult<BatchCreateRequest> {
    serde_json::from_value::<BatchCreateRequest>(request.clone())
        .map_err(|err| format!("batches request is invalid: {err}"))
}

pub fn batch_to_openai(batch: &Batch) -> Value {
    let mut value = serde_json::to_value(batch).unwrap_or(Value::Null);
    if let Value::Object(obj) = &mut value {
        obj.insert("object".to_string(), Value::String("batch".to_string()));
    }
    value
}

pub fn batch_list_response_to_openai(response: &BatchListResponse) -> Value {
    let mut obj = Map::<String, Value>::new();
    obj.insert("object".to_string(), Value::String("list".to_string()));

    let data: Vec<Value> = response.batches.iter().map(batch_to_openai).collect();
    obj.insert("data".to_string(), Value::Array(data));

    if let Some(has_more) = response.has_more {
        obj.insert("has_more".to_string(), Value::Bool(has_more));
    }

    let first_id = response
        .batches
        .first()
        .map(|batch| batch.id.trim().to_string())
        .filter(|id| !id.is_empty());
    if let Some(first_id) = first_id {
        obj.insert("first_id".to_string(), Value::String(first_id));
    }

    let last_id = response
        .batches
        .last()
        .map(|batch| batch.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .or_else(|| response.after.clone());
    if let Some(last_id) = last_id {
        obj.insert("last_id".to_string(), Value::String(last_id));
    }

    Value::Object(obj)
}

pub fn rerank_request_to_request(request: &Value) -> ParseResult<RerankRequest> {
    serde_json::from_value::<RerankRequest>(request.clone())
        .map_err(|err| format!("rerank request is invalid: {err}"))
}

pub fn rerank_response_to_openai(response: &RerankResponse) -> Value {
    let mut obj = Map::<String, Value>::new();

    if let Some(metadata) = response.provider_metadata.as_ref() {
        if let Some(id) = metadata.get("id") {
            obj.insert("id".to_string(), id.clone());
        }
        if let Some(meta) = metadata.get("meta") {
            obj.insert("meta".to_string(), meta.clone());
        }
    }

    let results: Vec<Value> = response
        .ranking
        .iter()
        .map(|result| {
            serde_json::json!({
                "index": result.index,
                "relevance_score": result.relevance_score,
            })
        })
        .collect();
    obj.insert("results".to_string(), Value::Array(results));

    Value::Object(obj)
}

#[derive(Debug, Clone)]
struct MultipartPart {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Bytes,
}

fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    if start >= haystack.len() {
        return None;
    }
    let first = needle[0];
    let mut pos = start;
    while pos + needle.len() <= haystack.len() {
        let rel = haystack[pos..].iter().position(|&b| b == first)?;
        pos += rel;
        if pos + needle.len() > haystack.len() {
            return None;
        }
        if &haystack[pos..pos + needle.len()] == needle {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn multipart_boundary(content_type: &str) -> ParseResult<String> {
    for part in content_type.split(';').map(str::trim) {
        if part.len() < "boundary=".len() {
            continue;
        }
        if !part[..].to_ascii_lowercase().starts_with("boundary=") {
            continue;
        }

        let value = part["boundary=".len()..].trim();
        if value.is_empty() {
            continue;
        }

        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);

        if unquoted.trim().is_empty() {
            continue;
        }

        return Ok(unquoted.to_string());
    }

    Err("multipart boundary is missing".to_string())
}

fn parse_multipart_form(content_type: &str, body: &Bytes) -> ParseResult<Vec<MultipartPart>> {
    let boundary = multipart_boundary(content_type)?;
    let boundary_marker = format!("--{boundary}");
    let boundary_bytes = boundary_marker.as_bytes();
    let delimiter = format!("\r\n{boundary_marker}");
    let delimiter_bytes = delimiter.as_bytes();

    let bytes = body.as_ref();
    let Some(mut cursor) = find_subslice(bytes, boundary_bytes, 0) else {
        return Err("multipart body missing boundary marker".to_string());
    };
    cursor += boundary_bytes.len();

    let mut parts = Vec::<MultipartPart>::new();
    loop {
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }

        let (headers_end, header_sep_len) =
            if let Some(idx) = find_subslice(bytes, b"\r\n\r\n", cursor) {
                (idx, 4)
            } else if let Some(idx) = find_subslice(bytes, b"\n\n", cursor) {
                (idx, 2)
            } else {
                return Err("multipart part missing header separator".to_string());
            };

        let headers_raw = String::from_utf8_lossy(&bytes[cursor..headers_end]);
        let mut name: Option<String> = None;
        let mut filename: Option<String> = None;
        let mut content_type: Option<String> = None;

        for line in headers_raw.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("content-disposition") {
                for item in value.split(';').map(str::trim) {
                    if let Some(value) = item.strip_prefix("name=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        name = Some(value.to_string());
                    } else if let Some(value) = item.strip_prefix("filename=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        filename = Some(value.to_string());
                    }
                }
            } else if key.eq_ignore_ascii_case("content-type") && !value.is_empty() {
                content_type = Some(value.to_string());
            }
        }

        let name =
            name.ok_or_else(|| "multipart part missing content-disposition name".to_string())?;
        let data_start = headers_end + header_sep_len;

        let Some(delim_pos) = find_subslice(bytes, delimiter_bytes, data_start) else {
            return Err("multipart part missing trailing boundary".to_string());
        };
        let data_end = delim_pos;

        let data = body.slice(data_start..data_end);
        parts.push(MultipartPart {
            name,
            filename,
            content_type,
            data,
        });

        cursor = delim_pos + delimiter_bytes.len();
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }
    }

    Ok(parts)
}

pub fn multipart_extract_text_field(
    content_type: &str,
    body: &Bytes,
    field_name: &str,
) -> ParseResult<Option<String>> {
    let parts = parse_multipart_form(content_type, body)?;
    for part in parts {
        if part.name != field_name {
            continue;
        }
        if part.filename.is_some() {
            continue;
        }
        let text = String::from_utf8_lossy(part.data.as_ref())
            .trim()
            .to_string();
        if text.is_empty() {
            return Ok(None);
        }
        return Ok(Some(text));
    }
    Ok(None)
}

pub fn audio_transcriptions_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<AudioTranscriptionRequest> {
    let mut file: Option<MultipartPart> = None;
    let mut model: Option<String> = None;
    let mut language: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<TranscriptionResponseFormat> = None;
    let mut temperature: Option<f32> = None;

    let parts = parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "file" => {
                file = Some(part);
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "language" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    language = Some(value);
                }
            }
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "response_format" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                response_format = match value.as_str() {
                    "json" => Some(TranscriptionResponseFormat::Json),
                    "text" => Some(TranscriptionResponseFormat::Text),
                    "srt" => Some(TranscriptionResponseFormat::Srt),
                    "verbose_json" => Some(TranscriptionResponseFormat::VerboseJson),
                    "vtt" => Some(TranscriptionResponseFormat::Vtt),
                    _ => None,
                };
            }
            "temperature" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if let Ok(parsed) = value.parse::<f32>() {
                    if parsed.is_finite() {
                        temperature = Some(parsed);
                    }
                }
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| "audio/transcriptions request missing file".to_string())?;
    let model = model.ok_or_else(|| "audio/transcriptions request missing model".to_string())?;

    let filename = file
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "audio".to_string());

    Ok(AudioTranscriptionRequest {
        audio: file.data.to_vec(),
        filename,
        media_type: file.content_type.clone(),
        model: Some(model),
        language,
        prompt,
        response_format,
        temperature,
        provider_options: None,
    })
}

pub fn audio_speech_request_to_request(request: &Value) -> ParseResult<SpeechRequest> {
    serde_json::from_value::<SpeechRequest>(request.clone())
        .map_err(|err| format!("audio/speech request is invalid: {err}"))
}

pub fn speech_response_format_to_content_type(
    format: Option<SpeechResponseFormat>,
) -> &'static str {
    match format {
        Some(SpeechResponseFormat::Mp3) => "audio/mpeg",
        Some(SpeechResponseFormat::Opus) => "audio/opus",
        Some(SpeechResponseFormat::Aac) => "audio/aac",
        Some(SpeechResponseFormat::Flac) => "audio/flac",
        Some(SpeechResponseFormat::Wav) => "audio/wav",
        Some(SpeechResponseFormat::Pcm) => "audio/pcm",
        None => "application/octet-stream",
    }
}

pub fn transcription_format_to_content_type(
    format: Option<TranscriptionResponseFormat>,
) -> (&'static str, bool) {
    match format {
        Some(TranscriptionResponseFormat::Text) => ("text/plain; charset=utf-8", false),
        Some(TranscriptionResponseFormat::Srt) => ("application/x-subrip", false),
        Some(TranscriptionResponseFormat::Vtt) => ("text/vtt", false),
        Some(TranscriptionResponseFormat::Json) => ("application/json", true),
        Some(TranscriptionResponseFormat::VerboseJson) => ("application/json", true),
        None => ("application/json", true),
    }
}

pub fn chat_completions_request_to_generate_request(
    request: &Value,
) -> ParseResult<GenerateRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "chat/completions request must be a JSON object".to_string())?;

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat/completions request missing model".to_string())?;

    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "chat/completions request missing messages".to_string())?;

    let mut out_messages = Vec::<Message>::new();
    for msg in messages {
        out_messages.push(parse_openai_chat_message(msg)?);
    }

    let mut out: GenerateRequest = out_messages.into();
    out.model = Some(model.to_string());

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64) {
        if temperature.is_finite() {
            out.temperature = Some(temperature as f32);
        }
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64) {
        if top_p.is_finite() {
            out.top_p = Some(top_p as f32);
        }
    }
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    if let Some(tools_value) = obj.get("tools") {
        out.tools = Some(parse_openai_tools(tools_value)?);
    }
    if let Some(tool_choice_value) = obj.get("tool_choice") {
        out.tool_choice = parse_openai_tool_choice(tool_choice_value)?;
    }

    let provider_options = parse_provider_options_from_openai_request(obj);
    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            serde_json::to_value(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(out)
}

pub fn completions_request_to_generate_request(request: &Value) -> ParseResult<GenerateRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "completions request must be a JSON object".to_string())?;

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "completions request missing model".to_string())?;

    if let Some(suffix) = obj
        .get("suffix")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|suffix| !suffix.is_empty())
    {
        return Err(format!("unsupported completions suffix: {suffix}"));
    }

    let prompt = match obj.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.to_string(),
        Some(Value::Array(items)) => {
            if items.len() > 1 {
                return Err("completions prompt arrays are not supported".to_string());
            }
            items
                .first()
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_default()
        }
        _ => return Err("completions prompt must be a string".to_string()),
    };

    let mut out: GenerateRequest = vec![Message::user(prompt)].into();
    out.model = Some(model.to_string());

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64) {
        if temperature.is_finite() {
            out.temperature = Some(temperature as f32);
        }
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64) {
        if top_p.is_finite() {
            out.top_p = Some(top_p as f32);
        }
    }
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    let provider_options = parse_provider_options_from_openai_request(obj);
    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            serde_json::to_value(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(out)
}

pub fn embeddings_request_to_texts(request: &Value) -> ParseResult<Vec<String>> {
    let obj = request
        .as_object()
        .ok_or_else(|| "embeddings request must be a JSON object".to_string())?;

    if let Some(format) = obj.get("encoding_format").and_then(Value::as_str) {
        let format = format.trim();
        if !format.is_empty() && format != "float" {
            return Err(format!("unsupported encoding_format: {format}"));
        }
    }

    let input = obj
        .get("input")
        .ok_or_else(|| "embeddings request missing input".to_string())?;

    match input {
        Value::String(text) => Ok(vec![text.clone()]),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                match item {
                    Value::String(text) => out.push(text.clone()),
                    _ => return Err(format!("embeddings input[{idx}] must be a string")),
                }
            }
            if out.is_empty() {
                return Err("embeddings request input must not be empty".to_string());
            }
            Ok(out)
        }
        _ => Err("embeddings request input must be a string or array of strings".to_string()),
    }
}

pub fn moderations_request_to_request(request: &Value) -> ParseResult<ModerationRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "moderations request must be a JSON object".to_string())?;

    let input = obj
        .get("input")
        .ok_or_else(|| "moderations request missing input".to_string())?;

    let input = match input {
        Value::String(text) => ModerationInput::Text(text.clone()),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let text = item
                    .as_str()
                    .ok_or_else(|| format!("moderations input[{idx}] must be a string"))?;
                out.push(text.to_string());
            }
            ModerationInput::TextArray(out)
        }
        other => ModerationInput::Raw(other.clone()),
    };

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    Ok(ModerationRequest {
        input,
        model,
        provider_options: None,
    })
}

pub fn moderation_response_to_openai(response: &ModerationResponse, fallback_id: &str) -> Value {
    let results = response
        .results
        .iter()
        .map(|result| {
            serde_json::json!({
                "flagged": result.flagged,
                "categories": result.categories,
                "category_scores": result.category_scores,
            })
        })
        .collect::<Vec<_>>();

    let mut out = Map::<String, Value>::new();
    out.insert(
        "id".to_string(),
        Value::String(
            response
                .id
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(fallback_id)
                .to_string(),
        ),
    );
    if let Some(model) = response.model.as_deref().filter(|v| !v.trim().is_empty()) {
        out.insert("model".to_string(), Value::String(model.to_string()));
    }
    out.insert("results".to_string(), Value::Array(results));
    Value::Object(out)
}

pub fn images_generation_request_to_request(
    request: &Value,
) -> ParseResult<ImageGenerationRequest> {
    serde_json::from_value::<ImageGenerationRequest>(request.clone()).map_err(|err| {
        format!("images/generations request cannot be parsed as ImageGenerationRequest: {err}")
    })
}

pub fn image_generation_response_to_openai(
    response: &ImageGenerationResponse,
    created: u64,
) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );

    let data = response
        .images
        .iter()
        .map(|image| match image {
            ImageSource::Url { url } => serde_json::json!({ "url": url }),
            ImageSource::Base64 { data, .. } => serde_json::json!({ "b64_json": data }),
        })
        .collect::<Vec<_>>();
    out.insert("data".to_string(), Value::Array(data));
    Value::Object(out)
}

pub fn responses_request_to_generate_request(request: &Value) -> ParseResult<GenerateRequest> {
    let chat = super::responses_shim::responses_request_to_chat_completions(request)
        .ok_or_else(|| "responses request cannot be mapped to chat/completions".to_string())?;
    let mut out = chat_completions_request_to_generate_request(&chat)?;

    let obj = request
        .as_object()
        .ok_or_else(|| "responses request must be a JSON object".to_string())?;

    let mut provider_options = ProviderOptions::default();
    if let Some(existing) = out
        .provider_options
        .as_ref()
        .and_then(|value| ProviderOptions::from_value(value).ok())
    {
        provider_options = existing;
    }

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

    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            serde_json::to_value(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(out)
}

pub fn embeddings_to_openai_response(embeddings: Vec<Vec<f32>>, model: &str) -> Value {
    fn safe_number(value: f32) -> Value {
        let num = serde_json::Number::from_f64(f64::from(value))
            .or_else(|| serde_json::Number::from_f64(0.0))
            .unwrap_or_else(|| serde_json::Number::from(0));
        Value::Number(num)
    }

    let mut data = Vec::<Value>::with_capacity(embeddings.len());
    for (index, embedding) in embeddings.into_iter().enumerate() {
        let vec = embedding.into_iter().map(safe_number).collect::<Vec<_>>();
        data.push(serde_json::json!({
            "object": "embedding",
            "index": index,
            "embedding": vec,
        }));
    }

    serde_json::json!({
        "object": "list",
        "data": data,
        "model": model,
    })
}
