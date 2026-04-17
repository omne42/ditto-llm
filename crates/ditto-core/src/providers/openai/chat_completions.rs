use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{Env, ProviderAuth, ProviderConfig};
use crate::contracts::{GenerateRequest, GenerateResponse};
use crate::error::Result;
use crate::llm_core::model::{LanguageModel, StreamResult};
use crate::providers::openai_chat_completions_core::{
    OpenAiChatCompletionsFacade, OpenAiChatCompletionsModelBehaviorResolver,
    OpenAiChatCompletionsRequestQuirks, apply_explicit_config_quirks, generate_chat_completions,
    resolve_request_quirks, stream_chat_completions,
};
use crate::providers::openai_compat_profile::OpenAiCompatibilityProfile;
use crate::providers::openai_like;

#[derive(Clone)]
pub struct OpenAIChatCompletions {
    client: openai_like::OpenAiLikeClient,
    compatibility_profile: OpenAiCompatibilityProfile,
    request_quirks: OpenAiChatCompletionsRequestQuirks,
    model_behavior_resolver: Option<Arc<OpenAiChatCompletionsModelBehaviorResolver>>,
}

impl OpenAIChatCompletions {
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = openai_like::OpenAiLikeClient::new(api_key);
        let compatibility_profile =
            OpenAiCompatibilityProfile::resolve("openai", &client.base_url, None);
        let request_quirks =
            OpenAiChatCompletionsRequestQuirks::from_profile(&compatibility_profile);
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
        let compatibility_profile = OpenAiCompatibilityProfile::resolve("openai", &base_url, None);
        let mut request_quirks =
            OpenAiChatCompletionsRequestQuirks::from_profile(&compatibility_profile);
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

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY"];
        let mut openai_config = config.clone();
        if openai_config.auth.is_none() {
            openai_config.auth = Some(ProviderAuth::ApiKeyEnv {
                keys: vec!["OPENAI_API_KEY".to_string()],
            });
        }
        let client =
            openai_like::OpenAiLikeClient::from_config_required(&openai_config, env, DEFAULT_KEYS)
                .await?;
        let compatibility_profile = OpenAiCompatibilityProfile::resolve(
            config.provider.as_deref().unwrap_or("openai"),
            &client.base_url,
            Some(config),
        );
        let mut request_quirks =
            OpenAiChatCompletionsRequestQuirks::from_profile(&compatibility_profile);
        apply_explicit_config_quirks(&mut request_quirks, config);
        Ok(Self {
            client,
            compatibility_profile,
            request_quirks,
            model_behavior_resolver: None,
        })
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        crate::providers::resolve_model_or_default(
            request
                .model
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            self.client.model.as_str(),
            "openai chat/completions",
            "set request.model or OpenAIChatCompletions::with_model",
        )
    }

    fn request_quirks_for_model(&self, model: &str) -> OpenAiChatCompletionsRequestQuirks {
        resolve_request_quirks(
            &self.compatibility_profile,
            self.request_quirks,
            self.model_behavior_resolver.as_ref(),
            model,
        )
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.client.apply_auth(req)
    }

    fn chat_completions_url(&self) -> String {
        self.client.endpoint("chat/completions")
    }
}

impl OpenAiChatCompletionsFacade for OpenAIChatCompletions {
    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn client(&self) -> &openai_like::OpenAiLikeClient {
        &self.client
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        self.resolve_model(request)
    }

    fn request_quirks_for_model(&self, model: &str) -> OpenAiChatCompletionsRequestQuirks {
        self.request_quirks_for_model(model)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.apply_auth(req)
    }

    fn chat_completions_url(&self) -> String {
        self.chat_completions_url()
    }
}

#[async_trait]
impl LanguageModel for OpenAIChatCompletions {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        generate_chat_completions(self, request).await
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        stream_chat_completions(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::Message;
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    #[tokio::test]
    async fn generate_hits_chat_completions_surface() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"service_tier\":\"default\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        json!({
                            "id": "chatcmpl_123",
                            "model": "gpt-4.1",
                            "choices": [{
                                "message": { "content": "ok" },
                                "finish_reason": "stop"
                            }],
                            "usage": {
                                "prompt_tokens": 1,
                                "completion_tokens": 1,
                                "total_tokens": 2
                            }
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIChatCompletions::new("sk-test")
            .with_base_url(server.url("/v1"))
            .with_model("gpt-4.1");

        let request = GenerateRequest {
            messages: vec![Message::user("hello")],
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
            provider_options: Some(crate::provider_options::ProviderOptionsEnvelope::from(
                json!({
                    "*": { "temperature": 0.4 },
                    "openai": { "service_tier": "default" },
                    "openai-compatible": { "service_tier": "flex" }
                }),
            )),
        };

        let response = client.generate(request).await?;
        mock.assert_async().await;
        assert_eq!(response.text(), "ok");
        Ok(())
    }
}
