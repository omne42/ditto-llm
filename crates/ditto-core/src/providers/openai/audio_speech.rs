use async_trait::async_trait;

use crate::capabilities::audio::SpeechModel;
use crate::config::{Env, ProviderConfig};
use crate::error::{DittoError, Result};
use crate::providers::{openai_audio_common, openai_like};
use crate::types::{SpeechRequest, SpeechResponse};

macro_rules! define_openai_like_speech {
    (
        $name:ident,
        provider = $provider:literal,
        default_keys = $default_keys:expr,
        from_config = $from_config:path,
        model_subject = $model_subject:literal,
        model_hint = $model_hint:literal $(,)?
    ) => {
        #[derive(Clone)]
        pub struct $name {
            client: openai_like::OpenAiLikeClient,
        }

        impl $name {
            pub fn new(api_key: impl Into<String>) -> Self {
                Self {
                    client: openai_like::OpenAiLikeClient::new(api_key),
                }
            }

            pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
                self.client = self.client.with_http_client(http);
                self
            }

            pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
                self.client = self.client.with_base_url(base_url);
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
                const DEFAULT_KEYS: &[&str] = $default_keys;
                Ok(Self {
                    client: $from_config(config, env, DEFAULT_KEYS).await?,
                })
            }

            fn resolve_model<'a>(&'a self, request: &'a SpeechRequest) -> Result<&'a str> {
                if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
                    return Ok(model);
                }
                if !self.client.model.trim().is_empty() {
                    return Ok(self.client.model.as_str());
                }
                Err(DittoError::provider_model_missing(
                    $model_subject,
                    $model_hint,
                ))
            }
        }

        #[async_trait]
        impl SpeechModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn speak(&self, request: SpeechRequest) -> Result<SpeechResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_audio_common::speak(self.provider(), &self.client, model, request).await
            }
        }
    };
}

#[cfg(feature = "provider-openai")]
define_openai_like_speech!(
    OpenAISpeech,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    model_subject = "openai speech",
    model_hint = "set request.model or OpenAISpeech::with_model",
);

#[cfg(feature = "provider-openai-compatible")]
define_openai_like_speech!(
    OpenAICompatibleSpeech,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    model_subject = "openai-compatible speech",
    model_hint = "set request.model or OpenAICompatibleSpeech::with_model",
);
