use async_trait::async_trait;

use crate::config::{Env, ProviderAuth, ProviderConfig};
use crate::model::{LanguageModel, StreamResult};
use crate::types::{GenerateRequest, ProviderOptionsEnvelope};
use crate::{GenerateResponse, OpenAICompatible, Result};

#[derive(Clone)]
pub struct OpenAIChatCompletions {
    inner: OpenAICompatible,
}

impl OpenAIChatCompletions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAICompatible::new(api_key),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.inner = self.inner.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.inner = self.inner.with_base_url(base_url);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.inner = self.inner.with_model(model);
        self
    }

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.inner = self.inner.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        let mut compat_config = config.clone();
        if compat_config.auth.is_none() {
            compat_config.auth = Some(ProviderAuth::ApiKeyEnv {
                keys: vec!["OPENAI_API_KEY".to_string()],
            });
        }
        Ok(Self {
            inner: OpenAICompatible::from_config(&compat_config, env).await?,
        })
    }

    fn remap_request_provider_options(mut request: GenerateRequest) -> Result<GenerateRequest> {
        let selected = request.provider_options_value_for("openai")?;
        request.provider_options = selected.map(ProviderOptionsEnvelope::from);
        Ok(request)
    }
}

#[async_trait]
impl LanguageModel for OpenAIChatCompletions {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        self.inner
            .generate(Self::remap_request_provider_options(request)?)
            .await
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        self.inner
            .stream(Self::remap_request_provider_options(request)?)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    #[test]
    fn remaps_bucketed_provider_options_to_openai_bucket_only() -> crate::Result<()> {
        let request = GenerateRequest {
            messages: vec![crate::Message::user("hello")],
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
            provider_options: Some(ProviderOptionsEnvelope::from(json!({
                "*": { "temperature": 0.4 },
                "openai": { "service_tier": "default" },
                "openai-compatible": { "service_tier": "flex" }
            }))),
        };

        let remapped = OpenAIChatCompletions::remap_request_provider_options(request)?;
        let selected = remapped
            .provider_options
            .expect("provider options should exist")
            .into_value();
        assert_eq!(
            selected,
            json!({
                "temperature": 0.4,
                "service_tier": "default"
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn generate_hits_chat_completions_surface() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .header("authorization", "Bearer sk-test");
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
            messages: vec![crate::Message::user("hello")],
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
            provider_options: Some(ProviderOptionsEnvelope::from(json!({
                "*": { "temperature": 0.4 },
                "openai": { "service_tier": "default" },
                "openai-compatible": { "service_tier": "flex" }
            }))),
        };

        let response = client.generate(request).await?;
        mock.assert_async().await;
        assert_eq!(response.text(), "ok");
        Ok(())
    }
}
