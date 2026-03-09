use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DittoError, Result};

use super::ProviderOptionsEnvelope;
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
    pub cache_creation_input_tokens: Option<u64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
    None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

impl ProviderOptions {
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value::<Self>(value.clone())
            .map_err(|err| DittoError::InvalidResponse(format!("invalid provider_options: {err}")))
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
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

impl GenerateRequest {
    pub fn with_provider_options(mut self, options: ProviderOptions) -> Result<Self> {
        self.provider_options = Some(ProviderOptionsEnvelope::from_options(options)?);
        Ok(self)
    }

    pub fn with_provider_response_format(
        mut self,
        provider: &str,
        response_format: ResponseFormat,
    ) -> Result<Self> {
        self.provider_options = Some(ProviderOptionsEnvelope::merge_response_format_for_provider(
            self.provider_options.take(),
            provider,
            response_format,
        )?);
        Ok(self)
    }

    pub fn provider_options_for(&self, provider: &str) -> Option<&Value> {
        self.provider_options
            .as_ref()
            .and_then(|options| options.provider_options_for(provider))
    }

    pub fn provider_options_value_for(&self, provider: &str) -> Result<Option<Value>> {
        match self.provider_options.as_ref() {
            Some(options) => options.provider_options_value_for(provider),
            None => Ok(None),
        }
    }

    pub fn parsed_provider_options_for(&self, provider: &str) -> Result<Option<ProviderOptions>> {
        match self.provider_options.as_ref() {
            Some(options) => options.parsed_provider_options_for(provider),
            None => Ok(None),
        }
    }

    pub fn parsed_provider_options(&self) -> Result<Option<ProviderOptions>> {
        match self.provider_options.as_ref() {
            Some(options) => options.parsed_provider_options(),
            None => Ok(None),
        }
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
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            logprobs: None,
            top_logprobs: None,
            user: None,
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
            prompt_cache_key: Some("cache_key".to_string()),
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
        let raw = ProviderOptionsEnvelope::from(json!({
            "*": { "parallel_tool_calls": false },
            "openai": { "parallel_tool_calls": true }
        }));

        let selected = raw.provider_options_value_for("openai")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(true));

        let selected = raw.provider_options_value_for("anthropic")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(false));
        Ok(())
    }

    #[test]
    fn provider_options_bucketed_supports_openai_compatible_alias_key() -> Result<()> {
        let raw = ProviderOptionsEnvelope::from(json!({
            "openai_compatible": { "parallel_tool_calls": true }
        }));

        let selected = raw
            .provider_options_value_for("openai-compatible")?
            .unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(true));
        Ok(())
    }

    #[test]
    fn provider_options_bucketed_supports_bedrock_and_vertex() -> Result<()> {
        let raw = ProviderOptionsEnvelope::from(json!({
            "bedrock": { "parallel_tool_calls": true },
            "vertex": { "parallel_tool_calls": false }
        }));

        let selected = raw.provider_options_value_for("bedrock")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(true));

        let selected = raw.provider_options_value_for("vertex")?.unwrap();
        let parsed = ProviderOptions::from_value(&selected)?;
        assert_eq!(parsed.parallel_tool_calls, Some(false));
        Ok(())
    }

    #[test]
    fn provider_options_bucketed_rejects_non_object_bucket() {
        let raw = ProviderOptionsEnvelope::from(json!({ "*": true }));
        let err = raw
            .provider_options_value_for("openai")
            .expect_err("should reject non-object buckets");
        match err {
            DittoError::InvalidResponse(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
