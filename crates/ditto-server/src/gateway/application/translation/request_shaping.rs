use bytes::Bytes;
use serde_json::{Map, Value};

use crate::gateway::multipart::{MultipartPart, parse_multipart_form};
use ditto_core::contracts::{
    ContentPart, GenerateRequest, ImageSource, Message, Role, Tool, ToolChoice,
};
use ditto_core::types::{
    AudioTranscriptionRequest, ImageEditRequest, ImageEditUpload, ImageGenerationRequest,
    ImageResponseFormat, ModerationInput, ModerationRequest, SpeechRequest, SpeechResponseFormat,
    TranscriptionResponseFormat, VideoContentVariant, VideoGenerationRequest, VideoListOrder,
    VideoListRequest, VideoReferenceUpload, VideoRemixRequest,
};

use super::ParseResult;
use super::openai_provider_options::apply_openai_request_provider_options;

pub(super) fn multipart_extract_text_field(
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

pub(super) fn audio_transcriptions_request_to_request(
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
                if let Ok(parsed) = value.parse::<f32>()
                    && parsed.is_finite()
                {
                    temperature = Some(parsed);
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

pub(super) fn audio_speech_request_to_request(request: &Value) -> ParseResult<SpeechRequest> {
    serde_json::from_value::<SpeechRequest>(request.clone())
        .map_err(|err| format!("audio/speech request is invalid: {err}"))
}

pub(super) fn speech_response_format_to_content_type(
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

pub(super) fn transcription_format_to_content_type(
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

pub(super) fn chat_completions_request_to_generate_request(
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

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64)
        && temperature.is_finite()
    {
        out.temperature = Some(temperature as f32);
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64)
        && top_p.is_finite()
    {
        out.top_p = Some(top_p as f32);
    }
    if let Some(seed) = obj.get("seed").and_then(Value::as_u64) {
        out.seed = Some(seed);
    }
    if let Some(presence_penalty) = obj.get("presence_penalty").and_then(Value::as_f64)
        && presence_penalty.is_finite()
    {
        out.presence_penalty = Some(presence_penalty as f32);
    }
    if let Some(frequency_penalty) = obj.get("frequency_penalty").and_then(Value::as_f64)
        && frequency_penalty.is_finite()
    {
        out.frequency_penalty = Some(frequency_penalty as f32);
    }
    if let Some(user) = obj
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.user = Some(user.to_string());
    }
    if let Some(logprobs) = obj.get("logprobs").and_then(Value::as_bool) {
        out.logprobs = Some(logprobs);
    }
    if let Some(top_logprobs) = obj.get("top_logprobs").and_then(Value::as_u64) {
        out.top_logprobs = Some(top_logprobs.min(u64::from(u32::MAX)) as u32);
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

    apply_openai_request_provider_options(&mut out, obj)?;

    Ok(out)
}

pub(super) fn completions_request_to_generate_request(
    request: &Value,
) -> ParseResult<GenerateRequest> {
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

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64)
        && temperature.is_finite()
    {
        out.temperature = Some(temperature as f32);
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64)
        && top_p.is_finite()
    {
        out.top_p = Some(top_p as f32);
    }
    if let Some(seed) = obj.get("seed").and_then(Value::as_u64) {
        out.seed = Some(seed);
    }
    if let Some(presence_penalty) = obj.get("presence_penalty").and_then(Value::as_f64)
        && presence_penalty.is_finite()
    {
        out.presence_penalty = Some(presence_penalty as f32);
    }
    if let Some(frequency_penalty) = obj.get("frequency_penalty").and_then(Value::as_f64)
        && frequency_penalty.is_finite()
    {
        out.frequency_penalty = Some(frequency_penalty as f32);
    }
    if let Some(user) = obj
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.user = Some(user.to_string());
    }
    if let Some(logprobs) = obj.get("logprobs").and_then(Value::as_u64)
        && logprobs > 0
    {
        out.logprobs = Some(true);
        out.top_logprobs = Some(logprobs.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    apply_openai_request_provider_options(&mut out, obj)?;

    Ok(out)
}

pub(super) fn embeddings_request_to_texts(request: &Value) -> ParseResult<Vec<String>> {
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

pub(super) fn moderations_request_to_request(request: &Value) -> ParseResult<ModerationRequest> {
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

pub(super) fn images_generation_request_to_request(
    request: &Value,
) -> ParseResult<ImageGenerationRequest> {
    serde_json::from_value::<ImageGenerationRequest>(request.clone()).map_err(|err| {
        format!("images/generations request cannot be parsed as ImageGenerationRequest: {err}")
    })
}

pub(super) fn images_edits_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<ImageEditRequest> {
    let mut prompt: Option<String> = None;
    let mut images = Vec::<ImageEditUpload>::new();
    let mut mask: Option<ImageEditUpload> = None;
    let mut model: Option<String> = None;
    let mut n: Option<u32> = None;
    let mut size: Option<String> = None;
    let mut response_format: Option<ImageResponseFormat> = None;

    let parts = parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "image" => images.push(image_edit_upload_from_part("image", part)),
            "mask" => {
                if mask.is_none() {
                    mask = Some(image_edit_upload_from_part("mask", part));
                }
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "n" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    n = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("images/edits request has invalid n: {value}"))?,
                    );
                }
            }
            "size" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    size = Some(value);
                }
            }
            "response_format" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    response_format = Some(parse_image_response_format(&value)?);
                }
            }
            _ => {}
        }
    }

    let prompt = prompt.ok_or_else(|| "images/edits request missing prompt".to_string())?;
    if images.is_empty() {
        return Err("images/edits request missing image".to_string());
    }

    Ok(ImageEditRequest {
        prompt,
        images,
        mask,
        model,
        n,
        size,
        response_format,
        provider_options: None,
    })
}

pub(super) fn responses_input_items_from_value(input: &Value) -> ParseResult<Vec<Value>> {
    match input {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(_) => Ok(vec![input.clone()]),
        Value::String(text) => Ok(vec![serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        })]),
        _ => Err("`input` must be a string, array, or object".to_string()),
    }
}

pub(super) fn videos_create_request_to_request(
    request: &Value,
) -> ParseResult<VideoGenerationRequest> {
    serde_json::from_value::<VideoGenerationRequest>(request.clone())
        .map_err(|err| format!("videos request cannot be parsed as VideoGenerationRequest: {err}"))
}

pub(super) fn videos_create_multipart_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<VideoGenerationRequest> {
    let mut prompt: Option<String> = None;
    let mut input_reference: Option<VideoReferenceUpload> = None;
    let mut model: Option<String> = None;
    let mut seconds: Option<u32> = None;
    let mut size: Option<String> = None;

    let parts = parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "input_reference" => {
                let filename = part
                    .filename
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "input_reference".to_string());
                input_reference = Some(VideoReferenceUpload {
                    data: part.data.to_vec(),
                    filename,
                    media_type: part.content_type.clone(),
                });
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "seconds" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    seconds = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("videos request has invalid seconds: {value}"))?,
                    );
                }
            }
            "size" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    size = Some(value);
                }
            }
            _ => {}
        }
    }

    Ok(VideoGenerationRequest {
        prompt: prompt.ok_or_else(|| "videos request missing prompt".to_string())?,
        input_reference,
        model,
        seconds,
        size,
        provider_options: None,
    })
}

pub(super) fn videos_remix_request_to_request(request: &Value) -> ParseResult<VideoRemixRequest> {
    serde_json::from_value::<VideoRemixRequest>(request.clone())
        .map_err(|err| format!("videos remix request cannot be parsed as VideoRemixRequest: {err}"))
}

pub(super) fn videos_content_variant_from_path(
    path_and_query: &str,
) -> ParseResult<Option<VideoContentVariant>> {
    let query = match path_and_query.split_once('?') {
        Some((_, query)) => query,
        None => return Ok(None),
    };

    let mut variant = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != "variant" || value.is_empty() {
            continue;
        }

        variant = Some(match value {
            "video" => VideoContentVariant::Video,
            "thumbnail" => VideoContentVariant::Thumbnail,
            "spritesheet" => VideoContentVariant::Spritesheet,
            _ => {
                return Err(format!(
                    "videos content request has unsupported variant: {value}"
                ));
            }
        });
    }

    Ok(variant)
}

pub(super) fn videos_list_request_from_path(path_and_query: &str) -> ParseResult<VideoListRequest> {
    let query = match path_and_query.split_once('?') {
        Some((_, query)) => query,
        None => {
            return Ok(VideoListRequest::default());
        }
    };

    let mut request = VideoListRequest::default();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "limit" if !value.is_empty() => {
                request.limit = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("videos list request has invalid limit: {value}"))?,
                );
            }
            "after" if !value.is_empty() => {
                request.after = Some(value.to_string());
            }
            "order" if !value.is_empty() => {
                request.order = Some(match value {
                    "asc" => VideoListOrder::Asc,
                    "desc" => VideoListOrder::Desc,
                    _ => {
                        return Err(format!(
                            "videos list request has unsupported order: {value}"
                        ));
                    }
                });
            }
            _ => {}
        }
    }

    Ok(request)
}

fn parse_image_response_format(value: &str) -> ParseResult<ImageResponseFormat> {
    match value.trim() {
        "url" => Ok(ImageResponseFormat::Url),
        "b64_json" => Ok(ImageResponseFormat::Base64Json),
        other => Err(format!(
            "images/edits request has unsupported response_format: {other}"
        )),
    }
}

fn image_edit_upload_from_part(field_name: &str, part: MultipartPart) -> ImageEditUpload {
    let filename = part
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| field_name.to_string());
    ImageEditUpload {
        data: part.data.to_vec(),
        filename,
        media_type: part.content_type.clone(),
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
        } else if let Some(function_call) = obj.get("function_call").and_then(Value::as_object)
            && let Some(part) = parse_openai_function_call(function_call)
        {
            parts.push(part);
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
                        if let Some(text) = obj.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            out.push(ContentPart::Text {
                                text: text.to_string(),
                            });
                            continue;
                        }

                        let ty = obj.get("type").and_then(Value::as_str).unwrap_or_default();
                        match ty {
                            "text" | "input_text" | "output_text" => {
                                if let Some(text) = obj.get("text").and_then(Value::as_str)
                                    && !text.is_empty()
                                {
                                    out.push(ContentPart::Text {
                                        text: text.to_string(),
                                    });
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
