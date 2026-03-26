#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use futures_util::TryStreamExt;
#[cfg(feature = "provider-openai")]
use serde::Deserialize;
#[cfg(feature = "provider-openai")]
use serde_json::{Map, Value};
#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use tokio::sync::mpsc;
#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use tokio_util::io::StreamReader;

#[cfg(feature = "provider-openai")]
use super::raw_responses::{
    OpenAIResponsesCompactionRequest, OpenAIResponsesRawEventStream, OpenAIResponsesRawRequest,
};
#[cfg(all(feature = "cap-llm-streaming", feature = "provider-openai"))]
use super::raw_responses::{OpenAIResponsesRawEvent, process_raw_responses_sse};
use crate::providers::openai_compat_profile::OpenAiCompatibilityProfile;
use crate::providers::openai_like;

use crate::config::{Env, ProviderConfig};
#[cfg(feature = "provider-openai")]
use crate::contracts::{
    ContentPart, FileSource, GenerateRequest, ImageSource, Message, Role, Tool, ToolChoice, Usage,
    Warning,
};
#[cfg(feature = "provider-openai")]
use crate::error::DittoError;
use crate::error::Result;

#[derive(Clone)]
pub struct OpenAI {
    pub(super) client: openai_like::OpenAiLikeClient,
    compatibility_profile: OpenAiCompatibilityProfile,
    tool_call_thought_signature_passthrough: Option<bool>,
}

#[cfg(feature = "provider-openai")]
pub(super) const OPENAI_RESPONSES_RESERVED_PROVIDER_OPTION_KEYS: &[&str] =
    &["reasoning_effort", "response_format", "parallel_tool_calls"];

#[cfg(feature = "provider-openai")]
pub(super) const OPENAI_RESPONSES_PROVIDER_OPTION_SCHEMA_KEYS: &[&str] = &[
    "instructions",
    "max_output_tokens",
    "previous_response_id",
    "store",
    "reasoning",
    "stream",
    "temperature",
    "top_p",
    "text",
    "tools",
    "tool_choice",
    "max_tool_calls",
    "metadata",
    "include",
    "service_tier",
    "truncation",
    "web_search_options",
    "caching",
    "expire_at",
    "context_management",
    "thinking",
];

#[cfg(feature = "provider-openai")]
pub(super) const TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR: &str = "__gts_";
#[cfg(feature = "provider-openai")]
pub(super) const OPENAI_RESPONSES_DUMMY_THOUGHT_SIGNATURE: &str =
    "skip_thought_signature_validator";

#[cfg(feature = "provider-openai")]
pub(super) fn split_tool_call_id_and_thought_signature(id: &str) -> (String, Option<String>) {
    let Some((base_id, hex)) = id.rsplit_once(TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR) else {
        return (id.to_string(), None);
    };
    if hex.len() % 2 != 0 {
        return (id.to_string(), None);
    }
    let mut bytes = Vec::<u8>::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let hex_pair = match std::str::from_utf8(chunk) {
            Ok(value) => value,
            Err(_) => return (id.to_string(), None),
        };
        let byte = match u8::from_str_radix(hex_pair, 16) {
            Ok(value) => value,
            Err(_) => return (id.to_string(), None),
        };
        bytes.push(byte);
    }
    let signature = match String::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return (id.to_string(), None),
    };
    if signature.trim().is_empty() {
        return (base_id.to_string(), None);
    }
    (base_id.to_string(), Some(signature))
}

#[cfg(feature = "provider-openai")]
pub(super) fn encode_tool_call_id_with_thought_signature(
    id: &str,
    thought_signature: Option<&str>,
) -> String {
    let (base_id, _) = split_tool_call_id_and_thought_signature(id);
    let Some(signature) = thought_signature
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return base_id;
    };
    let mut hex = String::with_capacity(signature.len() * 2);
    for byte in signature.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("{base_id}{TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR}{hex}")
}

impl OpenAI {
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = openai_like::OpenAiLikeClient::new(api_key);
        Self {
            compatibility_profile: OpenAiCompatibilityProfile::resolve(
                "openai",
                &client.base_url,
                None,
            ),
            client,
            tool_call_thought_signature_passthrough: None,
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client = self.client.with_base_url(base_url);
        self.compatibility_profile =
            OpenAiCompatibilityProfile::resolve("openai", &self.client.base_url, None);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.client = self.client.with_model(model);
        self
    }

    pub fn with_tool_call_thought_signature_passthrough(mut self, enabled: bool) -> Self {
        self.tool_call_thought_signature_passthrough = Some(enabled);
        self
    }

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.client = self.client.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY"];
        let client =
            openai_like::OpenAiLikeClient::from_config_required(config, env, DEFAULT_KEYS).await?;
        Ok(Self {
            compatibility_profile: OpenAiCompatibilityProfile::resolve(
                config.provider.as_deref().unwrap_or("openai"),
                &client.base_url,
                Some(config),
            ),
            client,
            // OPENAI-CONFIG-NO-ENV-QUIRKS: provider-side thought-signature
            // passthrough is driven by explicit config or request-derived
            // compatibility heuristics, never by ambient process env flags.
            tool_call_thought_signature_passthrough: config
                .openai_compatible
                .as_ref()
                .and_then(|explicit| explicit.send_tool_call_thought_signature),
        })
    }

    pub(super) fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.client.apply_auth(req)
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn responses_url(&self) -> String {
        self.client.endpoint("responses")
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn responses_compact_url(&self) -> String {
        format!("{}/compact", self.responses_url())
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::provider_model_missing(
            "openai",
            "set request.model or OpenAI::with_model",
        ))
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn tool_to_openai(tool: &Tool) -> Value {
        let mut out = Map::<String, Value>::new();
        out.insert("type".to_string(), Value::String("function".to_string()));
        out.insert("name".to_string(), Value::String(tool.name.clone()));
        if let Some(description) = &tool.description {
            out.insert(
                "description".to_string(),
                Value::String(description.clone()),
            );
        }
        out.insert("parameters".to_string(), tool.parameters.clone());
        out.insert(
            "strict".to_string(),
            Value::Bool(tool.strict.unwrap_or(true)),
        );
        Value::Object(out)
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn tool_choice_to_openai(choice: &ToolChoice) -> Value {
        match choice {
            ToolChoice::Auto => Value::String("auto".to_string()),
            ToolChoice::None => Value::String("none".to_string()),
            ToolChoice::Required => Value::String("required".to_string()),
            ToolChoice::Tool { name } => serde_json::json!({ "type": "function", "name": name }),
        }
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn tool_call_arguments_to_openai_string(arguments: &Value) -> String {
        match arguments {
            Value::String(raw) => {
                let raw = raw.trim();
                if raw.is_empty() {
                    "{}".to_string()
                } else {
                    raw.to_string()
                }
            }
            other => other.to_string(),
        }
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn should_send_function_call_thought_signature(&self, model: &str) -> bool {
        self.tool_call_thought_signature_passthrough
            .unwrap_or_else(|| {
                self.compatibility_profile
                    .should_send_tool_call_thought_signature(model)
            })
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn messages_to_input_with_quirks(
        messages: &[Message],
        include_function_call_thought_signature: bool,
    ) -> (Option<String>, Vec<Value>, Vec<Warning>) {
        let mut input = Vec::<Value>::new();
        let mut instructions = Vec::<String>::new();
        let mut warnings = Vec::<Warning>::new();

        for message in messages {
            match message.role {
                Role::System => {
                    let mut text = String::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text: chunk } => text.push_str(chunk),
                            other => warnings.push(Warning::Unsupported {
                                feature: "system_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported system content part: {other:?}"
                                )),
                            }),
                        }
                    }
                    let text = text.trim();
                    if !text.is_empty() {
                        instructions.push(text.to_string());
                    }
                }
                Role::User => {
                    let mut content = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                content.push(
                                    serde_json::json!({ "type": "input_text", "text": text }),
                                );
                            }
                            ContentPart::Image { source } => {
                                let image_url = match source {
                                    ImageSource::Url { url } => url.clone(),
                                    ImageSource::Base64 { media_type, data } => {
                                        format!("data:{media_type};base64,{data}")
                                    }
                                };
                                content.push(serde_json::json!({ "type": "input_image", "image_url": image_url }));
                            }
                            ContentPart::File {
                                filename,
                                media_type,
                                source,
                            } => {
                                if media_type != "application/pdf" {
                                    warnings.push(Warning::Unsupported {
                                        feature: "file".to_string(),
                                        details: Some(format!(
                                            "unsupported file media type for OpenAI Responses: {media_type}"
                                        )),
                                    });
                                    continue;
                                }

                                let item = match source {
                                    FileSource::Url { url } => {
                                        serde_json::json!({ "type": "input_file", "file_url": url })
                                    }
                                    FileSource::Base64 { data } => serde_json::json!({
                                        "type": "input_file",
                                        "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                                        "file_data": format!("data:{media_type};base64,{data}"),
                                    }),
                                    FileSource::FileId { file_id } => {
                                        serde_json::json!({ "type": "input_file", "file_id": file_id })
                                    }
                                };
                                content.push(item);
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "user_content_part".to_string(),
                                details: Some(format!("unsupported user content part: {other:?}")),
                            }),
                        }
                    }
                    if content.is_empty() {
                        continue;
                    }
                    input.push(serde_json::json!({ "role": "user", "content": content }));
                }
                Role::Assistant => {
                    let mut content = Vec::<Value>::new();
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                content.push(
                                    serde_json::json!({ "type": "output_text", "text": text }),
                                );
                            }
                            ContentPart::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                let (call_id, thought_signature) =
                                    split_tool_call_id_and_thought_signature(id);
                                if !content.is_empty() {
                                    input.push(serde_json::json!({ "role": "assistant", "content": content }));
                                    content = Vec::new();
                                }
                                let mut function_call = Map::<String, Value>::new();
                                function_call.insert(
                                    "type".to_string(),
                                    Value::String("function_call".to_string()),
                                );
                                function_call.insert("call_id".to_string(), Value::String(call_id));
                                function_call
                                    .insert("name".to_string(), Value::String(name.clone()));
                                function_call.insert(
                                    "arguments".to_string(),
                                    Value::String(Self::tool_call_arguments_to_openai_string(
                                        arguments,
                                    )),
                                );
                                if include_function_call_thought_signature {
                                    let thought_signature = thought_signature
                                        .as_deref()
                                        .unwrap_or(OPENAI_RESPONSES_DUMMY_THOUGHT_SIGNATURE);
                                    function_call.insert(
                                        "thought_signature".to_string(),
                                        Value::String(thought_signature.to_string()),
                                    );
                                }
                                input.push(Value::Object(function_call));
                            }
                            ContentPart::Reasoning { .. } => warnings.push(Warning::Unsupported {
                                feature: "reasoning".to_string(),
                                details: Some(
                                    "reasoning parts are not sent to OpenAI input".to_string(),
                                ),
                            }),
                            other => warnings.push(Warning::Unsupported {
                                feature: "assistant_content_part".to_string(),
                                details: Some(format!(
                                    "unsupported assistant content part: {other:?}"
                                )),
                            }),
                        }
                    }
                    if !content.is_empty() {
                        input.push(serde_json::json!({ "role": "assistant", "content": content }));
                    }
                }
                Role::Tool => {
                    for part in &message.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_call_id,
                                content,
                                ..
                            } => {
                                let (tool_call_id, _) =
                                    split_tool_call_id_and_thought_signature(tool_call_id);
                                input.push(serde_json::json!({
                                    "type": "function_call_output",
                                    "call_id": tool_call_id,
                                    "output": content,
                                }));
                            }
                            other => warnings.push(Warning::Unsupported {
                                feature: "tool_content_part".to_string(),
                                details: Some(format!("unsupported tool content part: {other:?}")),
                            }),
                        }
                    }
                }
            }
        }

        let instructions = if instructions.is_empty() {
            None
        } else {
            Some(instructions.join("\n\n"))
        };
        (instructions, input, warnings)
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("input_tokens").and_then(Value::as_u64);
            usage.cache_input_tokens = obj
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64)
                .or_else(|| {
                    obj.get("input_tokens_details")
                        .and_then(|details| details.get("cache_read_input_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| obj.get("cached_tokens").and_then(Value::as_u64))
                .or_else(|| obj.get("cache_read_input_tokens").and_then(Value::as_u64))
                .or_else(|| obj.get("cache_input_tokens").and_then(Value::as_u64));
            usage.cache_creation_input_tokens = obj
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    obj.get("input_tokens_details")
                        .and_then(|details| details.get("cache_creation_input_tokens"))
                        .and_then(Value::as_u64)
                })
                .or_else(|| obj.get("cache_write_input_tokens").and_then(Value::as_u64));
            usage.output_tokens = obj.get("output_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
    }

    #[cfg(feature = "provider-openai")]
    pub(super) fn build_responses_body(
        request: &GenerateRequest,
        model: &str,
        provider_options: &crate::provider_options::ProviderOptions,
        selected_provider_options: Option<&Value>,
        stream: bool,
        provider_options_context: &'static str,
        include_function_call_thought_signature: bool,
    ) -> Result<(Map<String, Value>, Vec<Warning>)> {
        let (instructions, input, mut warnings) = Self::messages_to_input_with_quirks(
            &request.messages,
            include_function_call_thought_signature,
        );

        if request.stop_sequences.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "stop_sequences".to_string(),
                details: Some("OpenAI Responses API stop sequences are not supported".to_string()),
            });
        }
        crate::types::warn_unsupported_generate_request_options(
            "OpenAI Responses API",
            request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        if let Some(instructions) = instructions {
            body.insert("instructions".to_string(), Value::String(instructions));
        }
        body.insert("input".to_string(), Value::Array(input));
        body.insert("stream".to_string(), Value::Bool(stream));
        body.insert("store".to_string(), Value::Bool(false));

        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            ) {
                body.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(max_tokens) = request.max_tokens {
            body.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_tokens.into()),
            );
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.0,
                1.0,
                &mut warnings,
            ) {
                body.insert("top_p".to_string(), Value::Number(value));
            }
        }

        if let Some(tools) = request.tools.as_ref() {
            if cfg!(feature = "cap-llm-tools") {
                let mapped = tools.iter().map(Self::tool_to_openai).collect();
                body.insert("tools".to_string(), Value::Array(mapped));
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-core built without tools feature".to_string()),
                });
            }
        }
        if let Some(tool_choice) = request.tool_choice.as_ref() {
            if cfg!(feature = "cap-llm-tools") {
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(tool_choice),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tool_choice".to_string(),
                    details: Some("ditto-core built without tools feature".to_string()),
                });
            }
        }

        apply_provider_options(&mut body, provider_options)?;
        crate::provider_options::merge_provider_options_into_body(
            &mut body,
            selected_provider_options,
            OPENAI_RESPONSES_RESERVED_PROVIDER_OPTION_KEYS,
            provider_options_context,
            &mut warnings,
        );

        Ok((body, warnings))
    }

    #[cfg(feature = "provider-openai")]
    pub async fn compact_responses_history_raw(
        &self,
        request: &OpenAIResponsesCompactionRequest<'_>,
    ) -> Result<Vec<Value>> {
        #[derive(Debug, Deserialize)]
        struct CompactionResponse {
            output: Vec<Value>,
        }

        let url = self.responses_compact_url();
        let req = self.client.http.post(url);
        let parsed = crate::provider_transport::send_checked_json::<CompactionResponse>(
            self.apply_auth(req).json(request),
        )
        .await?;
        Ok(parsed.output)
    }

    #[cfg(feature = "provider-openai")]
    pub async fn create_response_stream_raw(
        &self,
        request: &OpenAIResponsesRawRequest<'_>,
    ) -> Result<OpenAIResponsesRawEventStream> {
        #[cfg(not(feature = "cap-llm-streaming"))]
        {
            let _ = request;
            Err(DittoError::builder_capability_feature_missing(
                "openai",
                "streaming",
            ))
        }

        #[cfg(feature = "cap-llm-streaming")]
        {
            if !request.stream {
                return Err(crate::invalid_response!(
                    "error_detail.openai.responses_raw_stream_required"
                ));
            }

            let mut body = Map::<String, Value>::new();
            body.insert(
                "model".to_string(),
                Value::String(request.model.to_string()),
            );
            body.insert(
                "instructions".to_string(),
                Value::String(request.instructions.to_string()),
            );
            body.insert("input".to_string(), Value::Array(request.input.to_vec()));
            body.insert("store".to_string(), Value::Bool(request.store));
            body.insert("stream".to_string(), Value::Bool(true));
            body.insert(
                "parallel_tool_calls".to_string(),
                Value::Bool(request.parallel_tool_calls),
            );

            if let Some(tools) = request.tools.as_ref() {
                let mapped = tools.iter().map(Self::tool_to_openai).collect();
                body.insert("tools".to_string(), Value::Array(mapped));
            }
            if let Some(tool_choice) = request.tool_choice.as_ref() {
                body.insert(
                    "tool_choice".to_string(),
                    Self::tool_choice_to_openai(tool_choice),
                );
            }
            if request.reasoning_effort.is_some() || request.reasoning_summary.is_some() {
                let mut reasoning = Map::<String, Value>::new();
                if let Some(effort) = request.reasoning_effort {
                    reasoning.insert("effort".to_string(), serde_json::json!(effort));
                }
                if let Some(summary) = request.reasoning_summary {
                    reasoning.insert("summary".to_string(), serde_json::json!(summary));
                }
                body.insert("reasoning".to_string(), Value::Object(reasoning));
            }
            if let Some(response_format) = request.response_format.as_ref() {
                body.insert(
                    "response_format".to_string(),
                    serde_json::to_value(response_format)?,
                );
            }
            if !request.include.is_empty() {
                body.insert(
                    "include".to_string(),
                    Value::Array(request.include.iter().cloned().map(Value::String).collect()),
                );
            }
            if let Some(key) = request.prompt_cache_key.as_deref() {
                body.insert(
                    "prompt_cache_key".to_string(),
                    Value::String(key.to_string()),
                );
            }

            let url = self.responses_url();
            let req = self.client.http.post(url);
            let mut req = self.apply_auth(req).json(&body);
            for (name, value) in request.extra_headers.iter() {
                req = req.header(name, value);
            }
            req = req.header("Accept", "text/event-stream");
            let response = crate::provider_transport::send_checked(req).await?;

            let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
            let reader = StreamReader::new(byte_stream);
            let reader = tokio::io::BufReader::new(reader);

            let (tx_event, rx_event) = mpsc::channel::<Result<OpenAIResponsesRawEvent>>(512);
            let task = tokio::spawn(process_raw_responses_sse(reader, tx_event));
            Ok(OpenAIResponsesRawEventStream { rx_event, task })
        }
    }
}

#[cfg(feature = "provider-openai")]
pub(super) fn apply_provider_options(
    body: &mut Map<String, Value>,
    provider_options: &crate::provider_options::ProviderOptions,
) -> Result<()> {
    if provider_options.reasoning_effort.is_some() {
        let mut reasoning = Map::<String, Value>::new();
        if let Some(effort) = provider_options.reasoning_effort {
            reasoning.insert("effort".to_string(), serde_json::json!(effort));
        }
        body.insert("reasoning".to_string(), Value::Object(reasoning));
    }
    if let Some(response_format) = provider_options.response_format.as_ref() {
        body.insert(
            "response_format".to_string(),
            serde_json::to_value(response_format)?,
        );
    }
    if let Some(parallel_tool_calls) = provider_options.parallel_tool_calls {
        body.insert(
            "parallel_tool_calls".to_string(),
            Value::Bool(parallel_tool_calls),
        );
    }
    Ok(())
}

#[cfg(feature = "provider-openai")]
pub(super) fn sanitize_openai_responses_provider_options(
    selected_provider_options: Option<Value>,
    provider_options_context: &'static str,
) -> (Option<Value>, Vec<Warning>) {
    let Some(selected_provider_options) = selected_provider_options else {
        return (None, Vec::new());
    };

    let Some(obj) = selected_provider_options.as_object() else {
        // Keep old behavior for non-object values; downstream merge logic emits a warning.
        return (Some(selected_provider_options), Vec::new());
    };

    let mut out = Map::<String, Value>::new();
    let mut warnings = Vec::<Warning>::new();
    for (key, value) in obj {
        if OPENAI_RESPONSES_RESERVED_PROVIDER_OPTION_KEYS.contains(&key.as_str()) {
            continue;
        }
        if OPENAI_RESPONSES_PROVIDER_OPTION_SCHEMA_KEYS.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
            continue;
        }
        warnings.push(Warning::Unsupported {
            feature: provider_options_context.to_string(),
            details: Some(format!(
                "provider_options key {key:?} is not in the openai-responses schema; dropping"
            )),
        });
    }

    (
        if out.is_empty() {
            None
        } else {
            Some(Value::Object(out))
        },
        warnings,
    )
}

#[cfg(all(test, feature = "provider-openai"))]
mod tests {
    use super::*;
    use crate::config::{OpenAiCompatibleConfig, ProviderAuth};

    #[test]
    fn thought_signature_passthrough_defaults_to_explicit_input_heuristics() {
        let litellm = OpenAI::new("sk-test").with_base_url("https://litellm.example/v1");
        assert!(litellm.should_send_function_call_thought_signature("gemini-2.5-pro"));
        assert!(!litellm.should_send_function_call_thought_signature("gpt-5"));

        let plain = OpenAI::new("sk-test").with_base_url("https://api.openai.com/v1");
        assert!(!plain.should_send_function_call_thought_signature("gemini-2.5-pro"));
    }

    #[test]
    fn explicit_passthrough_override_can_disable_or_enable_heuristics() {
        let disabled = OpenAI::new("sk-test")
            .with_base_url("https://litellm.example/v1")
            .with_tool_call_thought_signature_passthrough(false);
        assert!(!disabled.should_send_function_call_thought_signature("gemini-2.5-pro"));

        let enabled = OpenAI::new("sk-test")
            .with_base_url("https://api.openai.com/v1")
            .with_tool_call_thought_signature_passthrough(true);
        assert!(enabled.should_send_function_call_thought_signature("gpt-5"));
    }

    #[tokio::test]
    async fn from_config_reads_explicit_passthrough_override() -> Result<()> {
        let config = ProviderConfig {
            auth: Some(ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_OPENAI_KEY".to_string()],
            }),
            openai_compatible: Some(OpenAiCompatibleConfig {
                family: None,
                send_prompt_cache_key: None,
                send_tool_call_thought_signature: Some(false),
            }),
            ..ProviderConfig::default()
        };
        let env = Env::parse_dotenv("DITTO_TEST_OPENAI_KEY=sk-test\n");

        let client = OpenAI::from_config(&config, &env).await?;
        assert!(!client.should_send_function_call_thought_signature("gemini-2.5-pro"));
        Ok(())
    }
}
