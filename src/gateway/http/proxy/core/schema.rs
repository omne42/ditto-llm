#[cfg(feature = "gateway-routing-advanced")]
use std::time::Duration;

fn extract_max_output_tokens(path: &str, value: &serde_json::Value) -> Option<u32> {
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

fn validate_openai_request_schema(
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

#[cfg(feature = "gateway-costing")]
fn clamp_u64_to_u32(value: u64) -> u32 {
    if value > u64::from(u32::MAX) {
        u32::MAX
    } else {
        value as u32
    }
}

fn estimate_tokens_from_bytes(body: &Bytes) -> u32 {
    let len = body.len();
    if len == 0 {
        return 0;
    }
    let estimate = (len.saturating_add(3) / 4) as u64;
    if estimate > u64::from(u32::MAX) {
        u32::MAX
    } else {
        estimate as u32
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ObservedUsage {
    input_tokens: Option<u64>,
    cache_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiUsageEnvelope {
    usage: Option<OpenAiUsagePayload>,
}

#[derive(serde::Deserialize)]
struct OpenAiUsagePayload {
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: Option<u64>,
    #[serde(default, alias = "completion_tokens")]
    output_tokens: Option<u64>,
    #[serde(default)]
    reasoning_tokens: Option<u64>,
    #[serde(default, alias = "prompt_tokens_details")]
    input_tokens_details: Option<OpenAiInputTokenDetails>,
    #[serde(default)]
    output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    completion_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiInputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
    #[serde(default, alias = "cache_creation_tokens")]
    cache_creation_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OpenAiOutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

fn extract_openai_usage_from_bytes(bytes: &Bytes) -> Option<ObservedUsage> {
    extract_openai_usage_from_slice(bytes.as_ref())
}

fn extract_openai_usage_from_slice(bytes: &[u8]) -> Option<ObservedUsage> {
    let usage = serde_json::from_slice::<OpenAiUsageEnvelope>(bytes)
        .ok()?
        .usage?;

    let input_tokens = usage.input_tokens;
    let output_tokens = usage.output_tokens;
    let reasoning_tokens = usage.reasoning_tokens.or_else(|| {
        usage
            .output_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .or_else(|| {
                usage
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|details| details.reasoning_tokens)
            })
    });
    let total_tokens = usage.total_tokens.or_else(|| {
        input_tokens.and_then(|input| output_tokens.map(|output| input.saturating_add(output)))
    });
    let cache_input_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    let cache_creation_input_tokens = usage.cache_creation_input_tokens.or_else(|| {
        usage
            .input_tokens_details
            .as_ref()
            .and_then(|details| details.cache_creation_tokens)
    });

    Some(ObservedUsage {
        input_tokens,
        cache_input_tokens,
        cache_creation_input_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
    })
}

fn sanitize_proxy_headers(headers: &mut HeaderMap, strip_authorization: bool) {
    if strip_authorization {
        headers.remove("authorization");
        headers.remove("x-api-key");
        headers.remove("x-litellm-api-key");
    }
    headers.remove("proxy-authorization");
    headers.remove("x-forwarded-authorization");
    headers.remove("connection");
    headers.remove("keep-alive");
    headers.remove("proxy-authenticate");
    headers.remove("proxy-connection");
    headers.remove("te");
    headers.remove("trailer");
    headers.remove("transfer-encoding");
    headers.remove("upgrade");
    headers.remove("x-ditto-virtual-key");
    headers.remove("x-ditto-protocol");
    headers.remove("x-ditto-cache-bypass");
    headers.remove("x-ditto-bypass-cache");
    headers.remove("content-length");
}

fn apply_backend_headers(headers: &mut HeaderMap, backend_headers: &HeaderMap) {
    for (name, value) in backend_headers.iter() {
        headers.insert(name, value.clone());
    }
}

fn generate_request_id() -> String {
    let seq = REQUEST_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("ditto-{ts_ms}-{seq}")
}

fn insert_request_id(headers: &mut HeaderMap, request_id: &str) {
    let value = match axum::http::HeaderValue::from_str(request_id) {
        Ok(value) => value,
        Err(_) => return,
    };
    headers.insert("x-request-id", value);
}

fn emit_json_log(state: &GatewayHttpState, event: &str, payload: serde_json::Value) {
    if !state.json_logs {
        return;
    }

    let payload = state.redactor.redact(payload);
    let record = serde_json::json!({
        "ts_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        "event": event,
        "payload": payload,
    });
    eprintln!("{record}");
}

#[cfg(feature = "sdk")]
fn emit_devtools_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    let Some(logger) = state.devtools.as_ref() else {
        return;
    };
    let payload = state.redactor.redact(payload);
    let _ = logger.log_event(kind, payload);
}

#[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
async fn append_audit_log(state: &GatewayHttpState, kind: &str, payload: serde_json::Value) {
    let payload = state.redactor.redact(payload);

    #[cfg(feature = "gateway-store-sqlite")]
    if let Some(store) = state.sqlite_store.as_ref() {
        let _ = store.append_audit_log(kind, payload.clone()).await;
    }

    #[cfg(feature = "gateway-store-redis")]
    if let Some(store) = state.redis_store.as_ref() {
        let _ = store.append_audit_log(kind, payload).await;
    }
}

type ProxyBodyStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

#[derive(Default)]
struct ProxyPermits {
    _proxy: Option<OwnedSemaphorePermit>,
    _backend: Option<OwnedSemaphorePermit>,
}

impl ProxyPermits {
    fn new(proxy: Option<OwnedSemaphorePermit>, backend: Option<OwnedSemaphorePermit>) -> Self {
        Self {
            _proxy: proxy,
            _backend: backend,
        }
    }

    fn is_empty(&self) -> bool {
        self._proxy.is_none() && self._backend.is_none()
    }

    fn take(&mut self) -> Self {
        Self {
            _proxy: self._proxy.take(),
            _backend: self._backend.take(),
        }
    }
}

struct ProxyBodyStreamWithPermit {
    inner: ProxyBodyStream,
    _permits: ProxyPermits,
}

impl futures_util::Stream for ProxyBodyStreamWithPermit {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.inner.as_mut().poll_next(cx)
    }
}

fn proxy_body_from_bytes_with_permit(bytes: Bytes, proxy_permits: ProxyPermits) -> Body {
    if proxy_permits.is_empty() {
        return Body::from(bytes);
    };

    let stream =
        futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(bytes) }).boxed();
    let stream = ProxyBodyStreamWithPermit {
        inner: stream,
        _permits: proxy_permits,
    };
    Body::from_stream(stream)
}
