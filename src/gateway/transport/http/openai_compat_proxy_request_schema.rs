pub(super) fn extract_max_output_tokens(path: &str, value: &serde_json::Value) -> Option<u32> {
    let key = if path.starts_with("/v1/responses") {
        "max_output_tokens"
    } else {
        "max_tokens"
    };

    value.get(key).and_then(|v| v.as_u64()).map(|v| {
        if v > u64::from(u32::MAX) {
            u32::MAX
        } else {
            v as u32
        }
    })
}

pub(super) fn validate_openai_request_schema(
    path_and_query: &str,
    body: &serde_json::Value,
) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);

    if path == "/v1/chat/completions" {
        return validate_openai_chat_completions_schema(body);
    }
    if path == "/v1/embeddings" {
        return validate_openai_embeddings_schema(body);
    }
    if path.starts_with("/v1/responses") {
        return validate_openai_responses_schema(body);
    }
    if path == "/v1/completions" {
        return validate_openai_completions_schema(body);
    }
    if path == "/v1/moderations" {
        return validate_openai_moderations_schema(body);
    }
    if path == "/v1/images/generations" {
        return validate_openai_images_generations_schema(body);
    }
    if path == "/v1/audio/speech" {
        return validate_openai_audio_speech_schema(body);
    }
    if path == "/v1/rerank" {
        return validate_openai_rerank_schema(body);
    }
    if path == "/v1/batches" {
        return validate_openai_batches_schema(body);
    }

    None
}

fn validate_openai_chat_completions_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(messages) = obj.get("messages").and_then(|value| value.as_array()) else {
        return Some("`messages` must be an array".to_string());
    };

    for (idx, message) in messages.iter().enumerate() {
        let Some(message) = message.as_object() else {
            return Some(format!("messages[{idx}] must be an object"));
        };

        let role = message
            .get("role")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if role.is_none() {
            return Some(format!("messages[{idx}].role must be a non-empty string"));
        }

        if !message.contains_key("content") {
            return Some(format!("messages[{idx}].content is required"));
        }
    }

    None
}

fn validate_openai_responses_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if !(input.is_string() || input.is_array() || input.is_object()) {
        return Some("`input` must be a string, array, or object".to_string());
    }

    None
}

fn validate_openai_embeddings_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if !(input.is_string() || input.is_array()) {
        return Some("`input` must be a string or array".to_string());
    }

    None
}

fn validate_openai_completions_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let Some(prompt) = obj.get("prompt") else {
        return Some("missing field `prompt`".to_string());
    };
    if !(prompt.is_string() || prompt.is_array()) {
        return Some("`prompt` must be a string or array".to_string());
    }

    None
}

fn validate_openai_moderations_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let Some(input) = obj.get("input") else {
        return Some("missing field `input`".to_string());
    };
    if input.is_null() {
        return Some("`input` must not be null".to_string());
    }
    if !(input.is_string() || input.is_array() || input.is_object()) {
        return Some("`input` must be a string, array, or object".to_string());
    }

    None
}

fn validate_openai_images_generations_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    match obj.get("prompt") {
        Some(serde_json::Value::String(prompt)) if !prompt.trim().is_empty() => None,
        Some(_) => Some("`prompt` must be a non-empty string".to_string()),
        None => Some("missing field `prompt`".to_string()),
    }
}

fn validate_openai_audio_speech_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let model = obj
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if model.is_none() {
        return Some("missing field `model`".to_string());
    }

    let input = obj
        .get("input")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if input.is_none() {
        return Some("missing field `input`".to_string());
    }

    let voice = obj
        .get("voice")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if voice.is_none() {
        return Some("missing field `voice`".to_string());
    }

    None
}

fn validate_openai_rerank_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let query = obj
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if query.is_none() {
        return Some("missing field `query`".to_string());
    }

    let Some(documents) = obj.get("documents") else {
        return Some("missing field `documents`".to_string());
    };
    if !documents.is_array() {
        return Some("`documents` must be an array".to_string());
    }

    None
}

fn validate_openai_batches_schema(body: &serde_json::Value) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some("request body must be a JSON object".to_string());
    };

    let input_file_id = obj
        .get("input_file_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if input_file_id.is_none() {
        return Some("missing field `input_file_id`".to_string());
    }

    let endpoint = obj
        .get("endpoint")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if endpoint.is_none() {
        return Some("missing field `endpoint`".to_string());
    }

    let completion_window = obj
        .get("completion_window")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if completion_window.is_none() {
        return Some("missing field `completion_window`".to_string());
    }

    None
}
