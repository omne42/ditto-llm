use std::sync::Arc;

#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
use futures_util::StreamExt;
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
use futures_util::stream;
#[cfg(feature = "provider-openai")]
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::ProviderConfig;
#[cfg(feature = "provider-openai")]
use crate::contracts::GenerateResponse;
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
use crate::contracts::StreamChunk;
use crate::contracts::{
    ContentPart, FileSource, FinishReason, GenerateRequest, ImageSource, Message, Role, Tool,
    ToolChoice, Usage, Warning,
};
use crate::error::Result;
#[cfg(feature = "provider-openai")]
use crate::llm_core::model::StreamResult;
use crate::providers::openai_compat_profile::{
    OpenAiCompatibilityProfile, OpenAiCompatibleModelBehavior, OpenAiProviderFamily,
};
#[cfg(feature = "provider-openai")]
use crate::providers::openai_like;

pub(crate) type OpenAiChatCompletionsModelBehaviorResolver =
    dyn Fn(&str) -> OpenAiCompatibleModelBehavior + Send + Sync;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenAiChatCompletionsRequestQuirks {
    pub(crate) family: OpenAiProviderFamily,
    pub(crate) assistant_tool_call_requires_reasoning_content: bool,
    pub(crate) tool_choice_required_supported: Option<bool>,
    pub(crate) assistant_tool_call_requires_thought_signature: bool,
    pub(crate) allow_prompt_cache_key: bool,
    pub(crate) force_assistant_tool_call_thought_signature: bool,
}

impl OpenAiChatCompletionsRequestQuirks {
    pub(crate) fn from_profile(profile: &OpenAiCompatibilityProfile) -> Self {
        Self {
            family: profile.family(),
            assistant_tool_call_requires_reasoning_content: profile
                .default_assistant_tool_call_requires_reasoning_content(),
            tool_choice_required_supported: None,
            assistant_tool_call_requires_thought_signature: false,
            allow_prompt_cache_key: profile.default_allow_prompt_cache_key(),
            force_assistant_tool_call_thought_signature: false,
        }
    }

    pub(crate) fn should_send_prompt_cache_key(self) -> bool {
        self.allow_prompt_cache_key
    }

    pub(crate) fn should_send_assistant_tool_call_thought_signature(self) -> bool {
        self.assistant_tool_call_requires_thought_signature
            || self.force_assistant_tool_call_thought_signature
    }
}

impl Default for OpenAiChatCompletionsRequestQuirks {
    fn default() -> Self {
        Self::from_profile(&OpenAiCompatibilityProfile::resolve(
            "",
            "https://api.openai.com/v1",
            None,
        ))
    }
}

#[cfg(feature = "provider-openai")]
pub(crate) trait OpenAiChatCompletionsFacade {
    fn provider_name(&self) -> &'static str;
    fn client(&self) -> &openai_like::OpenAiLikeClient;
    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str>;
    fn request_quirks_for_model(&self, model: &str) -> OpenAiChatCompletionsRequestQuirks;
    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder;
    fn chat_completions_url(&self) -> String;
}

fn assistant_tool_call_requires_reasoning_content(
    quirks: OpenAiChatCompletionsRequestQuirks,
) -> bool {
    quirks.assistant_tool_call_requires_reasoning_content
}

fn tool_choice_required_supported(quirks: OpenAiChatCompletionsRequestQuirks) -> Option<bool> {
    quirks.tool_choice_required_supported
}

fn apply_model_behavior(
    quirks: &mut OpenAiChatCompletionsRequestQuirks,
    behavior: OpenAiCompatibleModelBehavior,
) {
    quirks.assistant_tool_call_requires_reasoning_content |=
        behavior.assistant_tool_call_requires_reasoning_content;
    if let Some(supported) = behavior.tool_choice_required_supported {
        quirks.tool_choice_required_supported = Some(supported);
    }
}

pub(crate) fn apply_explicit_config_quirks(
    quirks: &mut OpenAiChatCompletionsRequestQuirks,
    config: &ProviderConfig,
) {
    let Some(explicit) = config.openai_compatible.as_ref() else {
        return;
    };
    if let Some(enabled) = explicit.send_prompt_cache_key {
        quirks.allow_prompt_cache_key = enabled;
    }
    if let Some(enabled) = explicit.send_tool_call_thought_signature {
        quirks.force_assistant_tool_call_thought_signature = enabled;
    }
}

pub(crate) fn resolve_request_quirks(
    profile: &OpenAiCompatibilityProfile,
    base_quirks: OpenAiChatCompletionsRequestQuirks,
    model_behavior_resolver: Option<&Arc<OpenAiChatCompletionsModelBehaviorResolver>>,
    model: &str,
) -> OpenAiChatCompletionsRequestQuirks {
    let mut quirks = base_quirks;
    quirks.family = profile.family();
    quirks.assistant_tool_call_requires_reasoning_content |=
        profile.default_assistant_tool_call_requires_reasoning_content();
    quirks.assistant_tool_call_requires_thought_signature |=
        profile.should_send_tool_call_thought_signature(model);
    if let Some(resolver) = model_behavior_resolver {
        apply_model_behavior(&mut quirks, resolver(model));
    } else {
        apply_model_behavior(&mut quirks, profile.model_behavior(model));
    }
    quirks
}

const TOOL_CALL_THOUGHT_SIGNATURE_SEPARATOR: &str = "__gts_";
pub(crate) const OPENAI_CHAT_COMPLETIONS_DUMMY_THOUGHT_SIGNATURE: &str =
    "skip_thought_signature_validator";

pub(crate) const OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS: &[&str] = &[
    "reasoning_effort",
    "response_format",
    "parallel_tool_calls",
    "prompt_cache_key",
];

#[derive(Debug, Clone)]
pub(crate) struct OpenAiChatCompletionsProviderOptionsSchemaResult {
    pub selected_provider_options: Option<serde_json::Value>,
    pub warnings: Vec<crate::contracts::Warning>,
}

const OPENAI_CHAT_COMPLETIONS_KNOWN_OPTION_KEYS: &[&str] = &[
    "model",
    "messages",
    "temperature",
    "max_tokens",
    "top_p",
    "seed",
    "presence_penalty",
    "frequency_penalty",
    "user",
    "logprobs",
    "top_logprobs",
    "stop",
    "stream",
    "stream_options",
    "tools",
    "tool_choice",
    "service_tier",
];

pub(crate) fn apply_openai_chat_completions_provider_options_schema(
    family: OpenAiProviderFamily,
    selected_provider_options: Option<serde_json::Value>,
    reserved_keys: &[&str],
    provider_options_context: &'static str,
) -> OpenAiChatCompletionsProviderOptionsSchemaResult {
    let Some(selected_provider_options) = selected_provider_options else {
        return OpenAiChatCompletionsProviderOptionsSchemaResult {
            selected_provider_options: None,
            warnings: Vec::new(),
        };
    };

    let Some(obj) = selected_provider_options.as_object() else {
        return OpenAiChatCompletionsProviderOptionsSchemaResult {
            selected_provider_options: Some(selected_provider_options),
            warnings: Vec::new(),
        };
    };

    let mut out = serde_json::Map::<String, serde_json::Value>::new();
    let mut warnings = Vec::<crate::contracts::Warning>::new();

    for (key, value) in obj {
        if reserved_keys.contains(&key.as_str()) {
            continue;
        }
        if OPENAI_CHAT_COMPLETIONS_KNOWN_OPTION_KEYS.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
            continue;
        }

        let mut handled_vendor_private_key = false;
        match family {
            OpenAiProviderFamily::OpenRouter => {
                if key == "provider" {
                    handled_vendor_private_key = true;
                    if value.is_object() {
                        out.insert("provider".to_string(), value.clone());
                    } else {
                        warnings.push(crate::contracts::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: "provider_options key \"provider\" for openrouter expects a JSON object; dropping".to_string(),
                        });
                    }
                }
            }
            OpenAiProviderFamily::DeepSeek => {
                if key == "thinking" || key == "thinking_config" {
                    handled_vendor_private_key = true;
                    if let Some(thinking) = sanitize_deepseek_thinking(value) {
                        out.insert("thinking".to_string(), thinking);
                        if key != "thinking" {
                            warnings.push(crate::contracts::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"thinking\" for deepseek"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::contracts::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for deepseek expects {{\"type\":\"enabled\"}} (or true); dropping"
                            ),
                        });
                    }
                }
            }
            OpenAiProviderFamily::MiniMax => {
                if key == "reasoning_split" || key == "reasoningSplit" {
                    handled_vendor_private_key = true;
                    if let Some(reasoning_split) = value.as_bool() {
                        out.insert(
                            "reasoning_split".to_string(),
                            serde_json::Value::Bool(reasoning_split),
                        );
                        if key != "reasoning_split" {
                            warnings.push(crate::contracts::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"reasoning_split\" for minimax"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::contracts::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for minimax expects a boolean; dropping"
                            ),
                        });
                    }
                }
            }
            OpenAiProviderFamily::Doubao => {
                if key == "thinking" || key == "thinking_config" {
                    handled_vendor_private_key = true;
                    if let Some(thinking) = sanitize_doubao_thinking(value) {
                        out.insert("thinking".to_string(), thinking);
                        if key != "thinking" {
                            warnings.push(crate::contracts::Warning::Compatibility {
                                feature: provider_options_context.to_string(),
                                details: format!(
                                    "provider_options key {key:?} mapped to \"thinking\" for doubao"
                                ),
                            });
                        }
                    } else {
                        warnings.push(crate::contracts::Warning::Compatibility {
                            feature: provider_options_context.to_string(),
                            details: format!(
                                "provider_options key {key:?} for doubao expects {{\"type\":\"enabled|disabled|auto\"}}; dropping"
                            ),
                        });
                    }
                }
            }
            OpenAiProviderFamily::OpenAi
            | OpenAiProviderFamily::Kimi
            | OpenAiProviderFamily::Qwen
            | OpenAiProviderFamily::Glm
            | OpenAiProviderFamily::LiteLlm
            | OpenAiProviderFamily::GenericOpenAiCompatible => {}
        }

        if handled_vendor_private_key {
            continue;
        }

        warnings.push(crate::contracts::Warning::Unsupported {
            feature: provider_options_context.to_string(),
            details: Some(format!(
                "provider_options key {key:?} is not in the openai-compatible schema for provider family {}; dropping",
                family.as_str()
            )),
        });
    }

    OpenAiChatCompletionsProviderOptionsSchemaResult {
        selected_provider_options: if out.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(out))
        },
        warnings,
    }
}

fn sanitize_deepseek_thinking(value: &serde_json::Value) -> Option<serde_json::Value> {
    if value.as_bool() == Some(true) {
        return Some(serde_json::json!({ "type": "enabled" }));
    }

    let obj = value.as_object()?;
    if obj.get("type").and_then(serde_json::Value::as_str) != Some("enabled") {
        return None;
    }
    Some(serde_json::json!({ "type": "enabled" }))
}

fn sanitize_doubao_thinking(value: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = value.as_object()?;
    let kind = obj.get("type").and_then(serde_json::Value::as_str)?;
    if !matches!(kind, "enabled" | "disabled" | "auto") {
        return None;
    }
    Some(serde_json::Value::Object(obj.clone()))
}

pub(crate) fn split_tool_call_id_and_thought_signature(id: &str) -> (String, Option<String>) {
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

pub(crate) fn encode_tool_call_id_with_thought_signature(
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

pub(crate) fn tool_to_openai(tool: &Tool, warnings: &mut Vec<Warning>) -> Value {
    if tool.strict.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "tool.strict".to_string(),
            details: Some("chat/completions does not support strict tool schemas".to_string()),
        });
    }

    let mut function = Map::<String, Value>::new();
    function.insert("name".to_string(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        function.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    function.insert("parameters".to_string(), tool.parameters.clone());

    let mut out = Map::<String, Value>::new();
    out.insert("type".to_string(), Value::String("function".to_string()));
    out.insert("function".to_string(), Value::Object(function));
    Value::Object(out)
}

pub(crate) fn tool_choice_to_openai(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => Value::String("auto".to_string()),
        ToolChoice::None => Value::String("none".to_string()),
        ToolChoice::Required => Value::String("required".to_string()),
        ToolChoice::Tool { name } => serde_json::json!({
            "type": "function",
            "function": { "name": name }
        }),
    }
}

fn tool_call_arguments_to_openai_string(arguments: &Value) -> String {
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

pub(crate) fn messages_to_chat_messages(
    messages: &[Message],
    _model: &str,
    quirks: OpenAiChatCompletionsRequestQuirks,
) -> (Vec<Value>, Vec<Warning>) {
    let mut out = Vec::<Value>::new();
    let mut warnings = Vec::<Warning>::new();
    let assistant_tool_call_requires_reasoning_content =
        assistant_tool_call_requires_reasoning_content(quirks);

    for message in messages {
        match message.role {
            Role::System => {
                let mut text = String::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text: chunk } => text.push_str(chunk),
                        other => warnings.push(Warning::Unsupported {
                            feature: "system_content_part".to_string(),
                            details: Some(format!("unsupported system content part: {other:?}")),
                        }),
                    }
                }
                if text.trim().is_empty() {
                    continue;
                }
                out.push(serde_json::json!({ "role": "system", "content": text }));
            }
            Role::User => {
                let mut texts = String::new();
                let mut parts = Vec::<Value>::new();
                let mut has_non_text = false;

                for part in &message.content {
                    match part {
                        ContentPart::Text { text } => {
                            if text.is_empty() {
                                continue;
                            }
                            texts.push_str(text);
                            parts.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                        ContentPart::Image { source } => {
                            has_non_text = true;
                            let image_url = match source {
                                ImageSource::Url { url } => url.clone(),
                                ImageSource::Base64 { media_type, data } => {
                                    format!("data:{media_type};base64,{data}")
                                }
                            };
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": { "url": image_url }
                            }));
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
                                        "unsupported file media type for openai-compatible: {media_type}"
                                    )),
                                });
                                continue;
                            }

                            has_non_text = true;
                            let part = match source {
                                FileSource::Url { url } => {
                                    warnings.push(Warning::Unsupported {
                                        feature: "file_url".to_string(),
                                        details: Some(format!(
                                            "openai-compatible chat messages do not support file URLs (url={url})"
                                        )),
                                    });
                                    continue;
                                }
                                FileSource::Base64 { data } => serde_json::json!({
                                    "type": "file",
                                    "file": {
                                        "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                                        "file_data": format!("data:{media_type};base64,{data}"),
                                    }
                                }),
                                FileSource::FileId { file_id } => serde_json::json!({
                                    "type": "file",
                                    "file": { "file_id": file_id }
                                }),
                            };
                            parts.push(part);
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "user_content_part".to_string(),
                            details: Some(format!("unsupported user content part: {other:?}")),
                        }),
                    }
                }

                if parts.is_empty() {
                    continue;
                }

                if has_non_text {
                    out.push(serde_json::json!({ "role": "user", "content": parts }));
                } else {
                    out.push(serde_json::json!({ "role": "user", "content": texts }));
                }
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut tool_calls = Vec::<Value>::new();
                for part in &message.content {
                    match part {
                        ContentPart::Text { text: chunk } => text.push_str(chunk),
                        ContentPart::Reasoning { text: chunk } => reasoning.push_str(chunk),
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            let (tool_call_id, thought_signature) =
                                split_tool_call_id_and_thought_signature(id);
                            let mut function = Map::<String, Value>::new();
                            function.insert("name".to_string(), Value::String(name.clone()));
                            function.insert(
                                "arguments".to_string(),
                                Value::String(tool_call_arguments_to_openai_string(arguments)),
                            );
                            if quirks.should_send_assistant_tool_call_thought_signature() {
                                let thought_signature = thought_signature
                                    .as_deref()
                                    .unwrap_or(OPENAI_CHAT_COMPLETIONS_DUMMY_THOUGHT_SIGNATURE);
                                function.insert(
                                    "thought_signature".to_string(),
                                    Value::String(thought_signature.to_string()),
                                );
                            }
                            let mut tool_call = Map::<String, Value>::new();
                            tool_call.insert("id".to_string(), Value::String(tool_call_id));
                            tool_call
                                .insert("type".to_string(), Value::String("function".to_string()));
                            tool_call.insert("function".to_string(), Value::Object(function));
                            tool_calls.push(Value::Object(tool_call));
                        }
                        other => warnings.push(Warning::Unsupported {
                            feature: "assistant_content_part".to_string(),
                            details: Some(format!("unsupported assistant content part: {other:?}")),
                        }),
                    }
                }

                if text.trim().is_empty() && tool_calls.is_empty() && reasoning.trim().is_empty() {
                    continue;
                }
                let has_tool_calls = !tool_calls.is_empty();

                let mut msg = Map::<String, Value>::new();
                msg.insert("role".to_string(), Value::String("assistant".to_string()));
                if text.trim().is_empty() {
                    msg.insert("content".to_string(), Value::Null);
                } else {
                    msg.insert("content".to_string(), Value::String(text));
                }
                if !tool_calls.is_empty() {
                    msg.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                if !reasoning.trim().is_empty()
                    || (has_tool_calls && assistant_tool_call_requires_reasoning_content)
                {
                    msg.insert("reasoning_content".to_string(), Value::String(reasoning));
                }
                out.push(Value::Object(msg));
            }
            Role::Tool => {
                for part in &message.content {
                    match part {
                        ContentPart::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } => {
                            if content.trim().is_empty() {
                                continue;
                            }
                            let (tool_call_id, _) =
                                split_tool_call_id_and_thought_signature(tool_call_id);
                            out.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_call_id,
                                "content": content,
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

    (out, warnings)
}

pub(crate) fn build_chat_completions_body(
    request: &GenerateRequest,
    model: &str,
    quirks: OpenAiChatCompletionsRequestQuirks,
    provider_options: &crate::provider_options::ProviderOptions,
    selected_provider_options: Option<&Value>,
    stream: bool,
    provider_options_context: &'static str,
) -> Result<(Map<String, Value>, Vec<Warning>)> {
    let (messages, mut warnings) = messages_to_chat_messages(&request.messages, model, quirks);

    let mut body = Map::<String, Value>::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert("stream".to_string(), Value::Bool(stream));
    if stream {
        body.insert(
            "stream_options".to_string(),
            serde_json::json!({ "include_usage": true }),
        );
    }

    if let Some(temperature) = request.temperature
        && let Some(value) = crate::utils::params::clamped_number_from_f32(
            "temperature",
            temperature,
            0.0,
            2.0,
            &mut warnings,
        )
    {
        body.insert("temperature".to_string(), Value::Number(value));
    }
    if let Some(max_tokens) = request.max_tokens {
        body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    }
    if let Some(top_p) = request.top_p
        && let Some(value) =
            crate::utils::params::clamped_number_from_f32("top_p", top_p, 0.0, 1.0, &mut warnings)
    {
        body.insert("top_p".to_string(), Value::Number(value));
    }
    if let Some(seed) = request.seed {
        body.insert("seed".to_string(), Value::Number(seed.into()));
    }
    if let Some(presence_penalty) = request.presence_penalty
        && let Some(value) = crate::utils::params::clamped_number_from_f32(
            "presence_penalty",
            presence_penalty,
            -2.0,
            2.0,
            &mut warnings,
        )
    {
        body.insert("presence_penalty".to_string(), Value::Number(value));
    }
    if let Some(frequency_penalty) = request.frequency_penalty
        && let Some(value) = crate::utils::params::clamped_number_from_f32(
            "frequency_penalty",
            frequency_penalty,
            -2.0,
            2.0,
            &mut warnings,
        )
    {
        body.insert("frequency_penalty".to_string(), Value::Number(value));
    }
    if let Some(user) = request
        .user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body.insert("user".to_string(), Value::String(user.to_string()));
    }
    match request.logprobs {
        Some(true) => {
            body.insert("logprobs".to_string(), Value::Bool(true));
        }
        Some(false) => {
            body.insert("logprobs".to_string(), Value::Bool(false));
        }
        None => {}
    }
    if let Some(top_logprobs) = request.top_logprobs {
        if request.logprobs == Some(false) {
            warnings.push(Warning::Compatibility {
                feature: "top_logprobs".to_string(),
                details: "top_logprobs requires logprobs=true; dropping".to_string(),
            });
        } else if top_logprobs == 0 || top_logprobs > 20 {
            warnings.push(Warning::Compatibility {
                feature: "top_logprobs".to_string(),
                details: format!(
                    "top_logprobs must be between 1 and 20 (got {top_logprobs}); dropping"
                ),
            });
        } else {
            body.insert("logprobs".to_string(), Value::Bool(true));
            body.insert(
                "top_logprobs".to_string(),
                Value::Number((top_logprobs as u64).into()),
            );
        }
    }
    if let Some(stops) = request.stop_sequences.as_ref() {
        let stops = crate::utils::params::sanitize_stop_sequences(stops, Some(4), &mut warnings);
        if !stops.is_empty() {
            body.insert(
                "stop".to_string(),
                Value::Array(stops.into_iter().map(Value::String).collect()),
            );
        }
    }

    if let Some(tools) = request.tools.as_ref() {
        if cfg!(feature = "cap-llm-tools") {
            let mapped = tools
                .iter()
                .map(|tool| tool_to_openai(tool, &mut warnings))
                .collect();
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
            if matches!(tool_choice, ToolChoice::Required)
                && matches!(tool_choice_required_supported(quirks), Some(false))
            {
                return Err(crate::invalid_response!(
                    "error_detail.openai.chat_completions_tool_choice_required_unsupported",
                    "model" => model
                ));
            }
            body.insert(
                "tool_choice".to_string(),
                tool_choice_to_openai(tool_choice),
            );
        } else {
            warnings.push(Warning::Unsupported {
                feature: "tool_choice".to_string(),
                details: Some("ditto-core built without tools feature".to_string()),
            });
        }
    }

    if let Some(effort) = provider_options.reasoning_effort {
        body.insert(
            "reasoning_effort".to_string(),
            serde_json::to_value(effort)?,
        );
    }
    if matches!(quirks.family, OpenAiProviderFamily::DeepSeek)
        && provider_options.reasoning_effort.is_some()
        && !body.contains_key("thinking")
    {
        body.remove("reasoning_effort");
        body.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "enabled" }),
        );
        warnings.push(Warning::Compatibility {
            feature: "reasoning_effort".to_string(),
            details: "mapped to thinking.type=enabled for deepseek compatibility".to_string(),
        });
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

    crate::provider_options::merge_provider_options_into_body(
        &mut body,
        selected_provider_options,
        OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS,
        provider_options_context,
        &mut warnings,
    );

    if let Some(prompt_cache_key) = provider_options
        .prompt_cache_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && quirks.should_send_prompt_cache_key()
    {
        body.insert(
            "prompt_cache_key".to_string(),
            Value::String(prompt_cache_key.to_string()),
        );
    }

    Ok((body, warnings))
}

pub(crate) fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some("function_call") => FinishReason::ToolCalls,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("error") => FinishReason::Error,
        _ => FinishReason::Unknown,
    }
}

pub(crate) fn parse_usage(value: &Value) -> Usage {
    let mut usage = Usage::default();
    if let Some(obj) = value.as_object() {
        let deepseek_cache_hit_tokens = obj.get("prompt_cache_hit_tokens").and_then(Value::as_u64);
        let deepseek_cache_miss_tokens =
            obj.get("prompt_cache_miss_tokens").and_then(Value::as_u64);

        usage.input_tokens = obj
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .or_else(
                || match (deepseek_cache_hit_tokens, deepseek_cache_miss_tokens) {
                    (Some(hit), Some(miss)) => Some(hit.saturating_add(miss)),
                    _ => None,
                },
            );
        usage.cache_input_tokens = obj
            .get("cached_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                obj.get("prompt_tokens_details")
                    .and_then(|details| details.get("cached_tokens"))
                    .and_then(Value::as_u64)
            })
            .or_else(|| {
                obj.get("prompt_tokens_details")
                    .and_then(|details| details.get("cache_read_input_tokens"))
                    .and_then(Value::as_u64)
            })
            .or_else(|| obj.get("cache_read_input_tokens").and_then(Value::as_u64))
            .or_else(|| obj.get("cache_input_tokens").and_then(Value::as_u64))
            .or(deepseek_cache_hit_tokens);
        usage.cache_creation_input_tokens = obj
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                obj.get("prompt_tokens_details")
                    .and_then(|details| details.get("cache_creation_input_tokens"))
                    .and_then(Value::as_u64)
            })
            .or_else(|| obj.get("cache_write_input_tokens").and_then(Value::as_u64));
        usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
        usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
    }
    usage.merge_total();
    usage
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "provider-openai")]
struct ChatCompletionsResponse {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
#[cfg(feature = "provider-openai")]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[cfg(feature = "provider-openai")]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCall>,
}

#[derive(Debug, Deserialize, Default)]
#[cfg(feature = "provider-openai")]
struct ChatFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
#[cfg(feature = "provider-openai")]
struct ChatToolCall {
    #[serde(default)]
    id: String,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
    #[serde(default)]
    function: ChatToolFunction,
}

#[derive(Debug, Deserialize, Default)]
#[cfg(feature = "provider-openai")]
struct ChatToolFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceChunk>,
    #[serde(default)]
    usage: Option<Value>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatChoiceChunk {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
    #[serde(default)]
    function_call: Option<ChatFunctionCallDelta>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatFunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
    #[serde(default)]
    function: Option<ChatToolFunctionDelta>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Deserialize, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct ChatToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default, alias = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct StreamToolCallState {
    id: Option<String>,
    name: Option<String>,
    thought_signature: Option<String>,
    started: bool,
    pending_arguments: String,
}

#[cfg(feature = "cap-llm-streaming")]
#[derive(Debug, Default)]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
struct StreamState {
    response_id: Option<String>,
    tool_calls: Vec<StreamToolCallState>,
    finish_reason: Option<String>,
}

#[cfg(feature = "cap-llm-streaming")]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
fn finalize_stream_state(state: &mut StreamState) -> Vec<StreamChunk> {
    let mut out = Vec::<StreamChunk>::new();
    let mut warnings = Vec::<Warning>::new();

    for (idx, slot) in state.tool_calls.iter_mut().enumerate() {
        if slot.started {
            continue;
        }

        let name = slot.name.as_deref().unwrap_or("").trim();
        let has_any_data = slot
            .id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || !name.is_empty()
            || !slot.pending_arguments.is_empty();

        if !has_any_data {
            continue;
        }

        let id = match slot.id.as_deref().filter(|value| !value.trim().is_empty()) {
            Some(id) => id.to_string(),
            None => {
                let synthesized = format!("call_{idx}");
                slot.id = Some(synthesized.clone());
                warnings.push(Warning::Compatibility {
                    feature: "tool_call.id".to_string(),
                    details: format!(
                        "stream ended before tool_call id was received; synthesizing {synthesized}"
                    ),
                });
                synthesized
            }
        };
        let id = encode_tool_call_id_with_thought_signature(&id, slot.thought_signature.as_deref());
        slot.id = Some(id.clone());

        if name.is_empty() {
            warnings.push(Warning::Compatibility {
                feature: "tool_call.name".to_string(),
                details: format!(
                    "stream ended before tool_call name was received for id={id}; dropping tool call"
                ),
            });
            slot.pending_arguments.clear();
            continue;
        }

        out.push(StreamChunk::ToolCallStart {
            id: id.clone(),
            name: name.to_string(),
        });
        slot.started = true;

        if !slot.pending_arguments.is_empty() {
            out.push(StreamChunk::ToolCallDelta {
                id,
                arguments_delta: std::mem::take(&mut slot.pending_arguments),
            });
        }
    }

    if !warnings.is_empty() {
        out.insert(0, StreamChunk::Warnings { warnings });
    }

    out
}

#[cfg(feature = "cap-llm-streaming")]
#[cfg(all(feature = "provider-openai", feature = "cap-llm-streaming"))]
fn parse_stream_data(state: &mut StreamState, data: &str) -> Result<(Vec<StreamChunk>, bool)> {
    let chunk = serde_json::from_str::<ChatCompletionsChunk>(data)?;
    let mut out = Vec::<StreamChunk>::new();
    let mut done = false;

    if state.response_id.is_none()
        && let Some(id) = chunk.id.as_deref().filter(|id| !id.trim().is_empty())
    {
        state.response_id = Some(id.to_string());
        out.push(StreamChunk::ResponseId { id: id.to_string() });
    }

    if let Some(usage) = chunk.usage.as_ref() {
        out.push(StreamChunk::Usage(parse_usage(usage)));
    }

    let Some(choice) = chunk.choices.first() else {
        return Ok((out, done));
    };

    if let Some(reasoning) = choice
        .delta
        .reasoning_content
        .as_deref()
        .or(choice.delta.reasoning.as_deref())
        && !reasoning.is_empty()
    {
        out.push(StreamChunk::ReasoningDelta {
            text: reasoning.to_string(),
        });
    }

    if let Some(content) = choice.delta.content.as_deref()
        && !content.is_empty()
    {
        out.push(StreamChunk::TextDelta {
            text: content.to_string(),
        });
    }

    if let Some(tool_calls) = choice.delta.tool_calls.as_ref() {
        for tool_call in tool_calls {
            let idx = tool_call.index;
            if state.tool_calls.len() <= idx {
                state
                    .tool_calls
                    .resize_with(idx + 1, StreamToolCallState::default);
            }
            let slot = &mut state.tool_calls[idx];

            if let Some(id) = tool_call
                .id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                slot.id = Some(id.to_string());
            }

            if let Some(function) = tool_call.function.as_ref() {
                if let Some(name) = function
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    slot.name = Some(name.to_string());
                }
                if let Some(thought_signature) = function
                    .thought_signature
                    .as_deref()
                    .or(tool_call.thought_signature.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    slot.thought_signature = Some(thought_signature.to_string());
                }
            }

            if !slot.started {
                let id = slot.id.as_deref().filter(|value| !value.trim().is_empty());
                let name = slot
                    .name
                    .as_deref()
                    .filter(|value| !value.trim().is_empty());
                if let (Some(id), Some(name)) = (id, name) {
                    let id = encode_tool_call_id_with_thought_signature(
                        id,
                        slot.thought_signature.as_deref(),
                    );
                    slot.id = Some(id.clone());
                    out.push(StreamChunk::ToolCallStart {
                        id: id.clone(),
                        name: name.to_string(),
                    });
                    slot.started = true;
                    if !slot.pending_arguments.is_empty() {
                        out.push(StreamChunk::ToolCallDelta {
                            id,
                            arguments_delta: std::mem::take(&mut slot.pending_arguments),
                        });
                    }
                }
            }

            if let Some(function) = tool_call.function.as_ref()
                && let Some(arguments) = function.arguments.as_deref()
            {
                if slot.started {
                    if let Some(id) = slot.id.as_deref() {
                        out.push(StreamChunk::ToolCallDelta {
                            id: id.to_string(),
                            arguments_delta: arguments.to_string(),
                        });
                    }
                } else {
                    slot.pending_arguments.push_str(arguments);
                }
            }
        }
    }

    if let Some(function_call) = choice.delta.function_call.as_ref() {
        if state.tool_calls.is_empty() {
            state.tool_calls.push(StreamToolCallState::default());
        }
        let slot = &mut state.tool_calls[0];

        if let Some(name) = function_call
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            slot.name = Some(name.to_string());
        }

        if !slot.started
            && let Some(name) = slot
                .name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        {
            let id = slot.id.clone().unwrap_or_else(|| "call_0".to_string());
            slot.id = Some(id.clone());
            out.push(StreamChunk::ToolCallStart {
                id: id.clone(),
                name: name.to_string(),
            });
            slot.started = true;
            if !slot.pending_arguments.is_empty() {
                out.push(StreamChunk::ToolCallDelta {
                    id,
                    arguments_delta: std::mem::take(&mut slot.pending_arguments),
                });
            }
        }

        if let Some(arguments) = function_call.arguments.as_deref() {
            if slot.started {
                if let Some(id) = slot.id.as_deref() {
                    out.push(StreamChunk::ToolCallDelta {
                        id: id.to_string(),
                        arguments_delta: arguments.to_string(),
                    });
                }
            } else {
                slot.pending_arguments.push_str(arguments);
            }
        }
    }

    if let Some(reason) = choice.finish_reason.as_deref() {
        state.finish_reason = Some(reason.to_string());
        out.push(StreamChunk::FinishReason(parse_finish_reason(Some(reason))));
        done = true;
    }

    Ok((out, done))
}

#[cfg(feature = "provider-openai")]
pub(crate) async fn generate_chat_completions<T>(
    adapter: &T,
    request: GenerateRequest,
) -> Result<GenerateResponse>
where
    T: OpenAiChatCompletionsFacade + ?Sized,
{
    let model = adapter.resolve_model(&request)?;
    let request_quirks = adapter.request_quirks_for_model(model);
    let raw_selected_provider_options =
        crate::provider_options::request_provider_options_value_for(
            &request,
            adapter.provider_name(),
        )?;
    let provider_options = raw_selected_provider_options
        .as_ref()
        .map(crate::provider_options::ProviderOptions::from_value_ref)
        .transpose()?
        .unwrap_or_default();
    let schema = apply_openai_chat_completions_provider_options_schema(
        request_quirks.family,
        raw_selected_provider_options,
        OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS,
        "generate.provider_options",
    );
    let selected_provider_options = schema.selected_provider_options;
    let (body, mut warnings) = build_chat_completions_body(
        &request,
        model,
        request_quirks,
        &provider_options,
        selected_provider_options.as_ref(),
        false,
        "generate.provider_options",
    )?;
    warnings.extend(schema.warnings);

    let url = adapter.chat_completions_url();
    let mut req = adapter.client().http.post(url);
    req = adapter.apply_auth(req);
    let parsed =
        crate::provider_transport::send_checked_json::<ChatCompletionsResponse>(req.json(&body))
            .await?;
    let choice = parsed.choices.first().ok_or_else(|| {
        crate::invalid_response!("error_detail.openai.chat_completions_response_no_choices")
    })?;

    let mut content = Vec::<ContentPart>::new();
    if let Some(reasoning) = choice
        .message
        .reasoning_content
        .as_deref()
        .or(choice.message.reasoning.as_deref())
        .filter(|value| !value.is_empty())
    {
        content.push(ContentPart::Reasoning {
            text: reasoning.to_string(),
        });
    }
    if let Some(text) = choice
        .message
        .content
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        content.push(ContentPart::Text {
            text: text.to_string(),
        });
    }
    match choice.message.tool_calls.as_ref() {
        Some(tool_calls) if !tool_calls.is_empty() => {
            for tool_call in tool_calls {
                let arguments_raw = tool_call.function.arguments.as_str();
                let context = format!("id={}", tool_call.id);
                let arguments = crate::contracts::parse_tool_call_arguments_json_or_string(
                    arguments_raw,
                    &context,
                    &mut warnings,
                );
                let thought_signature = tool_call
                    .function
                    .thought_signature
                    .as_deref()
                    .or(tool_call.thought_signature.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let id =
                    encode_tool_call_id_with_thought_signature(&tool_call.id, thought_signature);
                content.push(ContentPart::ToolCall {
                    id,
                    name: tool_call.function.name.clone(),
                    arguments,
                });
            }
        }
        _ => {
            if let Some(function_call) = choice.message.function_call.as_ref() {
                warnings.push(Warning::Compatibility {
                    feature: "tool_call.id".to_string(),
                    details:
                        "legacy function_call does not provide tool_call ids; synthesizing call_0"
                            .to_string(),
                });

                let name = function_call.name.trim();
                if !name.is_empty() {
                    let arguments_raw = function_call.arguments.as_str();
                    let context = format!("name={name}");
                    let arguments = crate::contracts::parse_tool_call_arguments_json_or_string(
                        arguments_raw,
                        &context,
                        &mut warnings,
                    );
                    content.push(ContentPart::ToolCall {
                        id: "call_0".to_string(),
                        name: name.to_string(),
                        arguments,
                    });
                } else {
                    warnings.push(Warning::Compatibility {
                        feature: "tool_call.name".to_string(),
                        details: "function_call.name is empty; dropping tool call".to_string(),
                    });
                }
            }
        }
    }

    let usage = parsed.usage.as_ref().map(parse_usage).unwrap_or_default();
    let finish_reason = parse_finish_reason(choice.finish_reason.as_deref());

    Ok(GenerateResponse {
        content,
        finish_reason,
        usage,
        warnings,
        provider_metadata: Some(serde_json::json!({ "id": parsed.id, "model": parsed.model })),
    })
}

#[cfg(feature = "provider-openai")]
pub(crate) async fn stream_chat_completions<T>(
    adapter: &T,
    request: GenerateRequest,
) -> Result<StreamResult>
where
    T: OpenAiChatCompletionsFacade + ?Sized,
{
    #[cfg(not(feature = "cap-llm-streaming"))]
    {
        let _ = adapter;
        let _ = request;
        return Err(DittoError::builder_capability_feature_missing(
            adapter.provider_name(),
            "streaming",
        ));
    }

    #[cfg(feature = "cap-llm-streaming")]
    {
        let model = adapter.resolve_model(&request)?;
        let request_quirks = adapter.request_quirks_for_model(model);
        let raw_selected_provider_options =
            crate::provider_options::request_provider_options_value_for(
                &request,
                adapter.provider_name(),
            )?;
        let provider_options = raw_selected_provider_options
            .as_ref()
            .map(crate::provider_options::ProviderOptions::from_value_ref)
            .transpose()?
            .unwrap_or_default();
        let schema = apply_openai_chat_completions_provider_options_schema(
            request_quirks.family,
            raw_selected_provider_options,
            OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS,
            "stream.provider_options",
        );
        let selected_provider_options = schema.selected_provider_options;
        let (body, mut warnings) = build_chat_completions_body(
            &request,
            model,
            request_quirks,
            &provider_options,
            selected_provider_options.as_ref(),
            true,
            "stream.provider_options",
        )?;
        warnings.extend(schema.warnings);

        let url = adapter.chat_completions_url();
        let req = adapter
            .client()
            .http
            .post(url)
            .header("Accept", "text/event-stream")
            .json(&body);
        let response = crate::provider_transport::send_checked(adapter.apply_auth(req)).await?;

        let (data_stream, buffer) = crate::session_transport::init_sse_stream(response, warnings);
        let stream = stream::unfold(
            (data_stream, buffer, StreamState::default(), false),
            |(mut data_stream, mut buffer, mut state, mut done)| async move {
                loop {
                    if let Some(item) = buffer.pop_front() {
                        return Some((item, (data_stream, buffer, state, done)));
                    }

                    if done {
                        return None;
                    }

                    let next = data_stream.next().await;
                    match next {
                        Some(Ok(data)) => match parse_stream_data(&mut state, &data) {
                            Ok((chunks, _is_done)) => {
                                for chunk in chunks {
                                    buffer.push_back(Ok(chunk));
                                }
                            }
                            Err(err) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                        },
                        Some(Err(err)) => {
                            done = true;
                            buffer.push_back(Err(err));
                        }
                        None => {
                            done = true;
                            for chunk in finalize_stream_state(&mut state) {
                                buffer.push_back(Ok(chunk));
                            }
                            let has_tool_calls = state.tool_calls.iter().any(|slot| slot.started);
                            if state.finish_reason.is_none() {
                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                    if has_tool_calls {
                                        FinishReason::ToolCalls
                                    } else {
                                        FinishReason::Stop
                                    },
                                )));
                            }
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}
