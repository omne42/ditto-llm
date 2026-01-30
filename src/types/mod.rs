use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{DittoError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: tool_call_id.into(),
                content: content.into(),
                is_error: None,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Url { url: String },
    Base64 { media_type: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSource {
    Url { url: String },
    Base64 { data: String },
    FileId { file_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
    },
    File {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        media_type: String,
        source: FileSource,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Reasoning {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Tool { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

impl Usage {
    pub fn merge_total(&mut self) {
        if self.total_tokens.is_some() {
            return;
        }
        if let (Some(input), Some(output)) = (self.input_tokens, self.output_tokens) {
            self.total_tokens = Some(input.saturating_add(output));
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Warning {
    Unsupported {
        feature: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    Clamped {
        parameter: String,
        original: f32,
        clamped_to: f32,
    },
    Compatibility {
        feature: String,
        details: String,
    },
    Other {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JsonSchemaFormat {
    pub name: String,
    pub schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonSchema { json_schema: JsonSchemaFormat },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProviderOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl ProviderOptions {
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value::<Self>(value.clone())
            .map_err(|err| DittoError::InvalidResponse(format!("invalid provider_options: {err}")))
    }
}

const PROVIDER_OPTIONS_BUCKETS: &[&str] = &[
    "*",
    "openai",
    "openai-compatible",
    "anthropic",
    "google",
    "cohere",
];

fn is_bucketed_provider_options(obj: &Map<String, Value>) -> bool {
    obj.keys()
        .any(|key| PROVIDER_OPTIONS_BUCKETS.contains(&key.as_str()))
}

pub(crate) fn provider_options_object_is_bucketed(obj: &Map<String, Value>) -> bool {
    is_bucketed_provider_options(obj)
}

pub(crate) fn select_provider_options<'a>(
    provider_options: Option<&'a Value>,
    provider: &str,
) -> Option<&'a Value> {
    let provider_options = provider_options?;
    let Some(obj) = provider_options.as_object() else {
        return Some(provider_options);
    };

    if is_bucketed_provider_options(obj) {
        if let Some(bucket) = obj.get(provider) {
            return Some(bucket);
        }
        if let Some(bucket) = obj.get("*") {
            return Some(bucket);
        }
        return None;
    }

    Some(provider_options)
}

pub(crate) fn select_provider_options_value(
    provider_options: Option<&Value>,
    provider: &str,
) -> Result<Option<Value>> {
    let Some(provider_options) = provider_options else {
        return Ok(None);
    };

    let Some(obj) = provider_options.as_object() else {
        return Ok(Some(provider_options.clone()));
    };

    if !is_bucketed_provider_options(obj) {
        return Ok(Some(provider_options.clone()));
    }

    let mut merged = Map::<String, Value>::new();
    let mut has_any = false;

    if let Some(value) = obj.get("*") {
        let Some(bucket) = value.as_object() else {
            return Err(DittoError::InvalidResponse(
                "invalid provider_options: bucket \"*\" must be a JSON object".to_string(),
            ));
        };
        for (key, value) in bucket {
            merged.insert(key.clone(), value.clone());
        }
        has_any = true;
    }

    if let Some(value) = obj.get(provider) {
        let Some(bucket) = value.as_object() else {
            return Err(DittoError::InvalidResponse(format!(
                "invalid provider_options: bucket {provider:?} must be a JSON object"
            )));
        };
        for (key, value) in bucket {
            merged.insert(key.clone(), value.clone());
        }
        has_any = true;
    }

    if !has_any {
        return Ok(None);
    }

    Ok(Some(Value::Object(merged)))
}

pub(crate) fn merge_provider_options_into_body(
    body: &mut Map<String, Value>,
    options: Option<&Value>,
    reserved_keys: &[&str],
    feature: &str,
    warnings: &mut Vec<Warning>,
) {
    let Some(options) = options else {
        return;
    };
    let Some(obj) = options.as_object() else {
        warnings.push(Warning::Unsupported {
            feature: feature.to_string(),
            details: Some("expected provider_options to be a JSON object".to_string()),
        });
        return;
    };

    for (key, value) in obj {
        if reserved_keys.contains(&key.as_str()) {
            continue;
        }

        if let Some(existing) = body.get_mut(key) {
            match (existing.as_object_mut(), value.as_object()) {
                (Some(existing_obj), Some(value_obj)) => {
                    for (nested_key, nested_value) in value_obj {
                        if existing_obj.contains_key(nested_key) {
                            warnings.push(Warning::Compatibility {
                                feature: feature.to_string(),
                                details: format!(
                                    "provider_options overrides {key}.{nested_key}; ignoring override"
                                ),
                            });
                            continue;
                        }
                        existing_obj.insert(nested_key.clone(), nested_value.clone());
                    }
                }
                _ => warnings.push(Warning::Compatibility {
                    feature: feature.to_string(),
                    details: format!("provider_options overrides {key}; ignoring override"),
                }),
            }
            continue;
        }

        body.insert(key.clone(), value.clone());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

impl GenerateRequest {
    pub fn with_provider_options(mut self, options: ProviderOptions) -> Result<Self> {
        self.provider_options = Some(serde_json::to_value(options)?);
        Ok(self)
    }

    pub fn provider_options_for(&self, provider: &str) -> Option<&Value> {
        select_provider_options(self.provider_options.as_ref(), provider)
    }

    pub fn provider_options_value_for(&self, provider: &str) -> Result<Option<Value>> {
        select_provider_options_value(self.provider_options.as_ref(), provider)
    }

    pub fn parsed_provider_options_for(&self, provider: &str) -> Result<Option<ProviderOptions>> {
        let selected = self.provider_options_value_for(provider)?;
        selected
            .as_ref()
            .map(ProviderOptions::from_value)
            .transpose()
    }

    pub fn parsed_provider_options(&self) -> Result<Option<ProviderOptions>> {
        self.provider_options
            .as_ref()
            .filter(|value| {
                value
                    .as_object()
                    .map(|obj| !is_bucketed_provider_options(obj))
                    .unwrap_or(true)
            })
            .map(ProviderOptions::from_value)
            .transpose()
    }
}

impl From<Vec<Message>> for GenerateRequest {
    fn from(messages: Vec<Message>) -> Self {
        Self {
            messages,
            model: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
            provider_options: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenerateResponse {
    #[serde(default)]
    pub content: Vec<ContentPart>,
    #[serde(default)]
    pub finish_reason: FinishReason,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

impl GenerateResponse {
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.content {
            if let ContentPart::Text { text } = part {
                out.push_str(text);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamChunk {
    Warnings { warnings: Vec<Warning> },
    ResponseId { id: String },
    TextDelta { text: String },
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, arguments_delta: String },
    ReasoningDelta { text: String },
    FinishReason(FinishReason),
    Usage(Usage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageResponseFormat {
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "b64_json")]
    Base64Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ImageResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageGenerationResponse {
    #[serde(default)]
    pub images: Vec<ImageSource>,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionResponseFormat {
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "srt")]
    Srt,
    #[serde(rename = "verbose_json")]
    VerboseJson,
    #[serde(rename = "vtt")]
    Vtt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTranscriptionRequest {
    pub audio: Vec<u8>,
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<TranscriptionResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioTranscriptionResponse {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeechResponseFormat {
    #[serde(rename = "mp3")]
    Mp3,
    #[serde(rename = "opus")]
    Opus,
    #[serde(rename = "aac")]
    Aac,
    #[serde(rename = "flac")]
    Flac,
    #[serde(rename = "wav")]
    Wav,
    #[serde(rename = "pcm")]
    Pcm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechRequest {
    pub input: String,
    pub voice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<SpeechResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeechResponse {
    #[serde(default)]
    pub audio: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModerationInput {
    Text(String),
    TextArray(Vec<String>),
    Raw(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationRequest {
    pub input: ModerationInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModerationResult {
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub categories: BTreeMap<String, bool>,
    #[serde(default)]
    pub category_scores: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModerationResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub results: Vec<ModerationResult>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RerankDocument {
    Text(String),
    Json(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RerankResult {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub relevance_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RerankResponse {
    #[serde(default)]
    pub ranking: Vec<RerankResult>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Validating,
    Failed,
    InProgress,
    Finalizing,
    Completed,
    Expired,
    Cancelling,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchRequestCounts {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub completed: u32,
    #[serde(default)]
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Batch {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: BatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_progress_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalizing_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelling_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_counts: Option<BatchRequestCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCreateRequest {
    pub input_file_id: String,
    pub endpoint: String,
    pub completion_window: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchResponse {
    pub batch: Batch,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchListResponse {
    #[serde(default)]
    pub batches: Vec<Batch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_options_roundtrip() -> Result<()> {
        let options = ProviderOptions {
            reasoning_effort: Some(ReasoningEffort::Medium),
            response_format: Some(ResponseFormat::JsonSchema {
                json_schema: JsonSchemaFormat {
                    name: "unit_test".to_string(),
                    schema: json!({ "type": "object" }),
                    strict: None,
                },
            }),
            parallel_tool_calls: Some(false),
        };

        let raw = serde_json::to_value(&options)?;
        let parsed = ProviderOptions::from_value(&raw)?;
        assert_eq!(parsed, options);
        Ok(())
    }

    #[test]
    fn provider_options_ignores_unknown_fields() -> Result<()> {
        let raw = json!({ "unknown": true });
        let parsed = ProviderOptions::from_value(&raw)?;
        assert_eq!(parsed, ProviderOptions::default());
        Ok(())
    }

    #[test]
    fn provider_options_bucketed_merges_provider_overrides_star() -> Result<()> {
        let raw = json!({
            "*": { "parallel_tool_calls": false },
            "openai": { "parallel_tool_calls": true }
        });

        let selected = select_provider_options_value(Some(&raw), "openai")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(true));

        let selected = select_provider_options_value(Some(&raw), "anthropic")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(false));
        Ok(())
    }

    #[test]
    fn provider_options_bucketed_rejects_non_object_bucket() {
        let raw = json!({ "*": true });
        let err = select_provider_options_value(Some(&raw), "openai")
            .expect_err("should reject non-object buckets");
        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
