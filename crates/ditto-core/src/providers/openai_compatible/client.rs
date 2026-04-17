use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like;

use crate::capabilities::context_cache::ContextCacheProfile;
#[cfg(feature = "cap-embedding")]
use crate::capabilities::embedding::EmbeddingModel;
use crate::capabilities::file::{FileContent, FileDeleteResponse, FileObject};
use crate::config::{Env, ProviderConfig};
use crate::contracts::ContentPart;
#[cfg(feature = "cap-llm-streaming")]
use crate::contracts::StreamChunk;
#[cfg(test)]
use crate::contracts::{FileSource, Message, Role, ToolChoice};
#[cfg(all(test, feature = "cap-llm-tools"))]
use crate::contracts::Tool;
use crate::contracts::{FinishReason, GenerateRequest, GenerateResponse, Usage, Warning};
use crate::error::Result;
use crate::llm_core::model::{LanguageModel, StreamResult};
#[cfg(test)]
use crate::providers::openai_chat_completions_core::{
    OPENAI_CHAT_COMPLETIONS_DUMMY_THOUGHT_SIGNATURE,
    messages_to_chat_messages as shared_messages_to_chat_messages,
    split_tool_call_id_and_thought_signature,
};
use crate::providers::openai_chat_completions_core::{
    OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS,
    OpenAiChatCompletionsModelBehaviorResolver, OpenAiChatCompletionsRequestQuirks,
    apply_explicit_config_quirks,
    build_chat_completions_body as shared_build_chat_completions_body,
    encode_tool_call_id_with_thought_signature, parse_finish_reason as shared_parse_finish_reason,
    parse_usage as shared_parse_usage, resolve_request_quirks,
};
#[cfg(test)]
use crate::providers::openai_compat_profile::OpenAiProviderFamily;
use crate::providers::openai_compat_profile::{
    OpenAiCompatibilityProfile, OpenAiCompatibleModelBehavior,
};
#[cfg(feature = "cap-llm-streaming")]
use futures_util::StreamExt;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::stream;

#[derive(Clone)]
pub struct OpenAICompatible {
    client: openai_like::OpenAiLikeClient,
    compatibility_profile: OpenAiCompatibilityProfile,
    request_quirks: OpenAiCompatibleRequestQuirks,
    model_behavior_resolver: Option<Arc<OpenAiCompatibleModelBehaviorResolver>>,
}

type OpenAiCompatibleRequestQuirks = OpenAiChatCompletionsRequestQuirks;
type OpenAiCompatibleModelBehaviorResolver = OpenAiChatCompletionsModelBehaviorResolver;
const OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS: &[&str] =
    OPENAI_CHAT_COMPLETIONS_RESERVED_PROVIDER_OPTION_KEYS;
#[cfg(test)]
const OPENAI_COMPAT_DUMMY_THOUGHT_SIGNATURE: &str = OPENAI_CHAT_COMPLETIONS_DUMMY_THOUGHT_SIGNATURE;

impl OpenAICompatible {
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = openai_like::OpenAiLikeClient::new(api_key);
        let compatibility_profile = OpenAiCompatibilityProfile::resolve("", &client.base_url, None);
        let request_quirks = OpenAiCompatibleRequestQuirks::from_profile(&compatibility_profile);
        Self {
            client,
            compatibility_profile,
            request_quirks,
            model_behavior_resolver: None,
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        self.client = self.client.with_base_url(base_url.clone());
        let compatibility_profile = OpenAiCompatibilityProfile::resolve(
            self.compatibility_profile.family().as_str(),
            &base_url,
            None,
        );
        let mut request_quirks =
            OpenAiCompatibleRequestQuirks::from_profile(&compatibility_profile);
        request_quirks.allow_prompt_cache_key = self.request_quirks.allow_prompt_cache_key;
        request_quirks.force_assistant_tool_call_thought_signature = self
            .request_quirks
            .force_assistant_tool_call_thought_signature;
        request_quirks.assistant_tool_call_requires_reasoning_content |= self
            .request_quirks
            .assistant_tool_call_requires_reasoning_content;
        request_quirks.tool_choice_required_supported =
            self.request_quirks.tool_choice_required_supported;
        self.compatibility_profile = compatibility_profile;
        self.request_quirks = request_quirks;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.client = self.client.with_model(model);
        self
    }

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.client = self.client.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub fn with_prompt_cache_key_passthrough(mut self, enabled: bool) -> Self {
        self.request_quirks.allow_prompt_cache_key = enabled;
        self
    }

    pub fn with_tool_call_thought_signature_passthrough(mut self, enabled: bool) -> Self {
        self.request_quirks
            .force_assistant_tool_call_thought_signature = enabled;
        self
    }

    pub fn with_assistant_tool_call_requires_reasoning_content(mut self, enabled: bool) -> Self {
        self.request_quirks
            .assistant_tool_call_requires_reasoning_content |= enabled;
        self
    }

    pub fn with_tool_choice_required_support(mut self, supported: Option<bool>) -> Self {
        self.request_quirks.tool_choice_required_supported = supported;
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_model_behavior_resolver<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&str) -> OpenAiCompatibleModelBehavior + Send + Sync + 'static,
    {
        self.model_behavior_resolver = Some(Arc::new(resolver));
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];
        let client =
            openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS).await?;
        let compatibility_profile = OpenAiCompatibilityProfile::resolve(
            config.provider.as_deref().unwrap_or(""),
            &client.base_url,
            Some(config),
        );
        let mut request_quirks =
            OpenAiCompatibleRequestQuirks::from_profile(&compatibility_profile);
        apply_explicit_config_quirks(&mut request_quirks, config);
        Ok(Self {
            client,
            compatibility_profile,
            request_quirks,
            model_behavior_resolver: None,
        })
    }

    pub fn context_cache_profile(&self) -> ContextCacheProfile {
        let model = self.client.model.as_str().trim();
        self.compatibility_profile
            .context_cache_profile((!model.is_empty()).then_some(model))
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.client.apply_auth(req)
    }

    fn chat_completions_url(&self) -> String {
        self.client.endpoint("chat/completions")
    }

    pub async fn upload_file(&self, filename: impl Into<String>, bytes: Vec<u8>) -> Result<String> {
        self.upload_file_with_purpose(filename, bytes, "assistants", None)
            .await
    }

    pub async fn upload_file_with_purpose(
        &self,
        filename: impl Into<String>,
        bytes: Vec<u8>,
        purpose: impl Into<String>,
        media_type: Option<&str>,
    ) -> Result<String> {
        self.client
            .upload_file_with_purpose(crate::capabilities::file::FileUploadRequest {
                filename: filename.into(),
                bytes,
                purpose: purpose.into(),
                media_type: media_type.map(|s| s.to_string()),
            })
            .await
    }

    pub async fn list_files(&self) -> Result<Vec<FileObject>> {
        self.client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        self.client.retrieve_file(file_id).await
    }

    pub async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        self.client.delete_file(file_id).await
    }

    pub async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        self.client.download_file_content(file_id).await
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        crate::providers::resolve_model_or_default(
            request.model.as_deref().filter(|m| !m.trim().is_empty()),
            self.client.model.as_str(),
            "openai-compatible",
            "set request.model or OpenAICompatible::with_model",
        )
    }

    fn request_quirks_for_model(&self, model: &str) -> OpenAiCompatibleRequestQuirks {
        resolve_request_quirks(
            &self.compatibility_profile,
            self.request_quirks,
            self.model_behavior_resolver.as_ref(),
            model,
        )
    }

    #[cfg(test)]
    fn tool_choice_to_openai(choice: &ToolChoice) -> Value {
        crate::providers::openai_chat_completions_core::tool_choice_to_openai(choice)
    }

    #[cfg(test)]
    fn messages_to_chat_messages(
        messages: &[Message],
        model: &str,
        quirks: OpenAiCompatibleRequestQuirks,
    ) -> (Vec<Value>, Vec<Warning>) {
        shared_messages_to_chat_messages(messages, model, quirks)
    }

    fn build_chat_completions_body(
        request: &GenerateRequest,
        model: &str,
        quirks: OpenAiCompatibleRequestQuirks,
        provider_options: &crate::provider_options::ProviderOptions,
        selected_provider_options: Option<&Value>,
        stream: bool,
        provider_options_context: &'static str,
    ) -> Result<(Map<String, Value>, Vec<Warning>)> {
        shared_build_chat_completions_body(
            request,
            model,
            quirks,
            provider_options,
            selected_provider_options,
            stream,
            provider_options_context,
        )
    }

    fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
        shared_parse_finish_reason(reason)
    }

    fn parse_usage(value: &Value) -> Usage {
        shared_parse_usage(value)
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use crate::error::DittoError;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn parse_usage_reads_cached_tokens_top_level() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cached_tokens": 7,
        }));
        assert_eq!(usage.cache_input_tokens, Some(7));
    }

    #[test]
    fn parse_usage_reads_cached_tokens_nested_prompt_tokens_details() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_tokens_details": { "cached_tokens": 5 },
        }));
        assert_eq!(usage.cache_input_tokens, Some(5));
    }

    #[test]
    fn parse_usage_reads_cache_read_input_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cache_read_input_tokens": 4,
        }));
        assert_eq!(usage.cache_input_tokens, Some(4));
    }

    #[test]
    fn parse_usage_reads_cache_write_input_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "cache_write_input_tokens": 3,
        }));
        assert_eq!(usage.cache_creation_input_tokens, Some(3));
    }

    #[tokio::test]
    async fn from_config_reads_explicit_passthrough_flags() -> Result<()> {
        let config = ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("test-model".to_string()),
            auth: Some(crate::config::ProviderAuth::ApiKeyEnv {
                keys: vec!["DITTO_TEST_OPENAI_COMPAT_KEY".to_string()],
            }),
            openai_compatible: Some(crate::config::OpenAiCompatibleConfig {
                family: None,
                send_prompt_cache_key: Some(true),
                send_tool_call_thought_signature: Some(true),
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([(
                "DITTO_TEST_OPENAI_COMPAT_KEY".to_string(),
                "sk-test".to_string(),
            )]),
        };

        let client = OpenAICompatible::from_config(&config, &env)
            .await?
            .with_base_url("https://proxy-2.example/v1");

        assert!(client.request_quirks.should_send_prompt_cache_key());
        assert!(
            client
                .request_quirks
                .should_send_assistant_tool_call_thought_signature()
        );
        Ok(())
    }

    #[test]
    fn request_quirks_for_model_keeps_family_defaults_without_runtime_injection() {
        let client = OpenAICompatible::new("sk-test").with_base_url("https://api.deepseek.com");

        let reasoner = client.request_quirks_for_model("deepseek-reasoner");
        assert!(!reasoner.assistant_tool_call_requires_reasoning_content);
        assert_eq!(reasoner.tool_choice_required_supported, None);

        let chat = client.request_quirks_for_model("deepseek-chat");
        assert!(!chat.assistant_tool_call_requires_reasoning_content);
        assert_eq!(chat.tool_choice_required_supported, None);
    }

    #[test]
    fn request_quirks_for_model_uses_runtime_injected_behavior_resolver() {
        let client = OpenAICompatible::new("sk-test").with_model_behavior_resolver(|model| {
            if model == "strict-model" {
                OpenAiCompatibleModelBehavior {
                    assistant_tool_call_requires_reasoning_content: true,
                    tool_choice_required_supported: Some(false),
                }
            } else {
                OpenAiCompatibleModelBehavior::default()
            }
        });

        let quirks = client.request_quirks_for_model("strict-model");
        assert!(quirks.assistant_tool_call_requires_reasoning_content);
        assert_eq!(quirks.tool_choice_required_supported, Some(false));
    }

    #[test]
    fn parse_usage_reads_deepseek_prompt_cache_hit_tokens_alias() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_cache_hit_tokens": 6,
            "prompt_cache_miss_tokens": 4
        }));
        assert_eq!(usage.cache_input_tokens, Some(6));
        assert_eq!(usage.input_tokens, Some(10));
    }

    #[test]
    fn parse_usage_derives_prompt_tokens_from_deepseek_hit_miss_when_missing_prompt_tokens() {
        let usage = OpenAICompatible::parse_usage(&json!({
            "completion_tokens": 2,
            "total_tokens": 12,
            "prompt_cache_hit_tokens": 7,
            "prompt_cache_miss_tokens": 3
        }));
        assert_eq!(usage.cache_input_tokens, Some(7));
        assert_eq!(usage.input_tokens, Some(10));
    }

    #[test]
    fn build_body_suppresses_prompt_cache_key_by_default_and_keeps_stream_usage() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::provider_options::ProviderOptions {
            prompt_cache_key: Some("thread-123".to_string()),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, _warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "gpt-4.1",
            OpenAiCompatibleRequestQuirks::default(),
            &provider_options,
            Some(&selected),
            true,
            "test.provider_options",
        )?;

        assert!(
            body.get("prompt_cache_key").is_none(),
            "prompt_cache_key should be suppressed by default for compatibility"
        );
        assert_eq!(
            body.get("stream_options")
                .and_then(Value::as_object)
                .and_then(|opts| opts.get("include_usage"))
                .and_then(Value::as_bool),
            Some(true)
        );

        Ok(())
    }

    #[test]
    fn build_body_includes_prompt_cache_key_when_quirk_enables_it() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::provider_options::ProviderOptions {
            prompt_cache_key: Some("thread-123".to_string()),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, _warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "gpt-4.1",
            OpenAiCompatibleRequestQuirks {
                allow_prompt_cache_key: true,
                ..Default::default()
            },
            &provider_options,
            Some(&selected),
            false,
            "test.provider_options",
        )?;

        assert_eq!(
            body.get("prompt_cache_key").and_then(Value::as_str),
            Some("thread-123")
        );
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(false));
        assert!(
            body.get("stream_options").is_none(),
            "non-streaming request should not include stream_options"
        );

        Ok(())
    }

    #[test]
    fn provider_options_schema_drops_unknown_keys_and_keeps_known_openai_fields() {
        let selected = serde_json::json!({
            "stream": true,
            "unknown_private_flag": true
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::Kimi,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({ "stream": true }))
        );
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Unsupported { feature, details }
                if feature == "test.provider_options"
                    && details.as_deref().is_some_and(|msg| msg.contains("unknown_private_flag"))
        )));
    }

    #[test]
    fn provider_options_schema_maps_minimax_reasoning_split_alias() {
        let selected = serde_json::json!({
            "reasoningSplit": true
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::MiniMax,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({ "reasoning_split": true }))
        );
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "test.provider_options"
                    && details.contains("\"reasoningSplit\"")
                    && details.contains("reasoning_split")
        )));
    }

    #[test]
    fn provider_options_schema_keeps_openrouter_provider_object() {
        let selected = serde_json::json!({
            "provider": {
                "order": ["Google AI Studio"],
                "allow_fallbacks": false
            }
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::OpenRouter,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(
            schema.selected_provider_options,
            Some(serde_json::json!({
                "provider": {
                    "order": ["Google AI Studio"],
                    "allow_fallbacks": false
                }
            }))
        );
        assert!(schema.warnings.is_empty());
    }

    #[test]
    fn provider_options_schema_drops_non_object_openrouter_provider() {
        let selected = serde_json::json!({
            "provider": "Google AI Studio"
        });
        let schema = apply_openai_compatible_provider_options_schema(
            OpenAiProviderFamily::OpenRouter,
            Some(selected),
            OPENAI_COMPAT_RESERVED_PROVIDER_OPTION_KEYS,
            "test.provider_options",
        );

        assert_eq!(schema.selected_provider_options, None);
        assert!(schema.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "test.provider_options"
                    && details.contains("\"provider\" for openrouter expects a JSON object")
        )));
    }

    #[test]
    fn build_body_maps_deepseek_reasoning_effort_to_thinking() -> Result<()> {
        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let provider_options = crate::provider_options::ProviderOptions {
            reasoning_effort: Some(crate::provider_options::ReasoningEffort::High),
            ..Default::default()
        };
        let selected = serde_json::to_value(&provider_options)?;

        let (body, warnings) = OpenAICompatible::build_chat_completions_body(
            &request,
            "deepseek-chat",
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::DeepSeek,
                ..Default::default()
            },
            &provider_options,
            Some(&selected),
            false,
            "test.provider_options",
        )?;

        assert!(
            body.get("reasoning_effort").is_none(),
            "deepseek requests should not send reasoning_effort directly"
        );
        assert_eq!(
            body.get("thinking"),
            Some(&serde_json::json!({ "type": "enabled" }))
        );
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, details }
                if feature == "reasoning_effort"
                    && details.contains("deepseek")
        )));
        Ok(())
    }

    #[test]
    fn messages_to_chat_messages_preserves_assistant_reasoning_for_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![
                ContentPart::Reasoning {
                    text: "chain-of-thought".to_string(),
                },
                ContentPart::ToolCall {
                    id: "call_1".to_string(),
                    name: "thread".to_string(),
                    arguments: json!({"op":"state"}),
                },
            ],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], "gpt-4o", Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings
                .iter()
                .all(|warning| !matches!(warning, Warning::Unsupported { feature, .. } if feature == "reasoning")));

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("chain-of-thought")
        );
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_keeps_reasoning_only_assistant_message() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::Reasoning {
                text: "thinking-only".to_string(),
            }],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], "gpt-4o", Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("thinking-only")
        );
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("tool_calls").is_none());
    }

    #[test]
    fn messages_to_chat_messages_skips_empty_reasoning_for_tool_calls_by_default() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) =
            OpenAICompatible::messages_to_chat_messages(&[assistant], "gpt-4o", Default::default());
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert!(msg.get("reasoning_content").is_none());
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_adds_empty_reasoning_for_kimi_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant],
            "moonshot-v1-8k",
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::Kimi,
                assistant_tool_call_requires_reasoning_content: true,
                tool_choice_required_supported: None,
                assistant_tool_call_requires_thought_signature: false,
                allow_prompt_cache_key: false,
                force_assistant_tool_call_thought_signature: false,
            },
        );
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(msg.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(msg.get("content"), Some(&Value::Null));
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("")
        );
        assert!(msg.get("tool_calls").is_some());
    }

    #[test]
    fn messages_to_chat_messages_adds_dummy_thought_signature_for_litellm_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant],
            "gpt-4o",
            OpenAiCompatibleRequestQuirks {
                assistant_tool_call_requires_thought_signature: true,
                ..Default::default()
            },
        );
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        let tool_calls = msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool calls array");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_1")
        );
        assert_eq!(
            tool_calls[0]
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("thought_signature"))
                .and_then(Value::as_str),
            Some(OPENAI_COMPAT_DUMMY_THOUGHT_SIGNATURE)
        );
    }

    #[test]
    fn messages_to_chat_messages_replays_encoded_thought_signature_and_normalizes_ids() {
        let encoded_id = encode_tool_call_id_with_thought_signature("call_abc", Some("hi"));
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: encoded_id.clone(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };
        let tool = Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: encoded_id,
                content: "ok".to_string(),
                is_error: None,
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant, tool],
            "gpt-4o",
            OpenAiCompatibleRequestQuirks {
                assistant_tool_call_requires_thought_signature: true,
                ..Default::default()
            },
        );
        assert_eq!(messages.len(), 2);
        assert!(warnings.is_empty());

        let assistant_msg = messages[0].as_object().expect("assistant message object");
        let tool_calls = assistant_msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("tool calls array");
        assert_eq!(
            tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_abc")
        );
        assert_eq!(
            tool_calls[0]
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("thought_signature"))
                .and_then(Value::as_str),
            Some("hi")
        );

        let tool_msg = messages[1].as_object().expect("tool message object");
        assert_eq!(
            tool_msg.get("tool_call_id").and_then(Value::as_str),
            Some("call_abc")
        );
    }

    #[test]
    fn messages_to_chat_messages_adds_empty_reasoning_for_deepseek_reasoner_tool_calls() {
        let assistant = Message {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: "call_1".to_string(),
                name: "workspace".to_string(),
                arguments: json!({"op":"help"}),
            }],
        };

        let (messages, warnings) = OpenAICompatible::messages_to_chat_messages(
            &[assistant],
            "deepseek-reasoner",
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::DeepSeek,
                assistant_tool_call_requires_reasoning_content: true,
                ..Default::default()
            },
        );
        assert_eq!(messages.len(), 1);
        assert!(warnings.is_empty());

        let msg = messages[0].as_object().expect("assistant message object");
        assert_eq!(
            msg.get("reasoning_content").and_then(Value::as_str),
            Some("")
        );
        assert!(msg.get("tool_calls").is_some());
    }

    #[cfg(feature = "cap-llm-tools")]
    #[test]
    fn build_body_rejects_deepseek_reasoner_required_tool_choice() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }),
            strict: None,
        };
        let mut request = GenerateRequest::from(vec![Message::user("hi")]);
        request.tools = Some(vec![tool]);
        request.tool_choice = Some(ToolChoice::Required);
        let provider_options = crate::provider_options::ProviderOptions::default();
        let selected = serde_json::to_value(&provider_options).expect("provider options json");

        let err = OpenAICompatible::build_chat_completions_body(
            &request,
            "deepseek-reasoner",
            OpenAiCompatibleRequestQuirks {
                family: OpenAiProviderFamily::DeepSeek,
                tool_choice_required_supported: Some(false),
                ..Default::default()
            },
            &provider_options,
            Some(&selected),
            false,
            "test.provider_options",
        )
        .expect_err("deepseek-reasoner should reject tool_choice=required");

        assert!(matches!(
            err,
            DittoError::InvalidResponse(ref message)
                if message.as_catalog().map(|message| message.code())
                    == Some("error_detail.openai.chat_completions_tool_choice_required_unsupported")
        ));
    }
}
