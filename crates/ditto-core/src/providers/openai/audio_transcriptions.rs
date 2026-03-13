use async_trait::async_trait;

use crate::capabilities::audio::AudioTranscriptionModel;
use crate::config::{Env, ProviderConfig};
use crate::foundation::error::{DittoError, Result};
use crate::providers::{openai_audio_common, openai_like};
use crate::types::{AudioTranscriptionRequest, AudioTranscriptionResponse};

macro_rules! define_openai_like_audio_transcription {
    (
        $name:ident,
        provider = $provider:literal,
        default_keys = $default_keys:expr,
        from_config = $from_config:path,
        missing_model_error = $missing_model_error:literal $(,)?
    ) => {
        #[derive(Clone)]
        pub struct $name {
            pub(super) client: openai_like::OpenAiLikeClient,
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

            pub(super) fn resolve_model<'a>(
                &'a self,
                request: &'a AudioTranscriptionRequest,
            ) -> Result<&'a str> {
                if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
                    return Ok(model);
                }
                if !self.client.model.trim().is_empty() {
                    return Ok(self.client.model.as_str());
                }
                Err(DittoError::InvalidResponse(
                    $missing_model_error.to_string(),
                ))
            }
        }

        #[async_trait]
        impl AudioTranscriptionModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn transcribe(
                &self,
                request: AudioTranscriptionRequest,
            ) -> Result<AudioTranscriptionResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_audio_common::transcribe($provider, &self.client, model, request).await
            }
        }
    };
}

#[cfg(feature = "openai")]
define_openai_like_audio_transcription!(
    OpenAIAudioTranscription,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error = "openai audio transcription model is not set (set request.model or OpenAIAudioTranscription::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_audio_transcription!(
    OpenAICompatibleAudioTranscription,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible audio transcription model is not set (set request.model or OpenAICompatibleAudioTranscription::with_model)",
);
