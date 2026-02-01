use async_trait::async_trait;

use super::openai_audio_common;
use super::openai_like;

use crate::audio::{AudioTranscriptionModel, AudioTranslationModel, SpeechModel};
use crate::profile::{Env, ProviderConfig};
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAICompatibleAudioTranscription {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAICompatibleAudioTranscription {
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

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS)
                .await?,
        })
    }

    fn resolve_model<'a>(&'a self, request: &'a AudioTranscriptionRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai-compatible audio transcription model is not set (set request.model or OpenAICompatibleAudioTranscription::with_model)"
                .to_string(),
        ))
    }
}

#[async_trait]
impl AudioTranscriptionModel for OpenAICompatibleAudioTranscription {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn transcribe(
        &self,
        request: AudioTranscriptionRequest,
    ) -> Result<AudioTranscriptionResponse> {
        let model = self.resolve_model(&request)?.to_string();
        openai_audio_common::transcribe("openai-compatible", &self.client, model, request).await
    }
}

#[async_trait]
impl AudioTranslationModel for OpenAICompatibleAudioTranscription {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn translate(
        &self,
        request: AudioTranscriptionRequest,
    ) -> Result<AudioTranscriptionResponse> {
        let model = self.resolve_model(&request)?.to_string();
        openai_audio_common::translate("openai-compatible", &self.client, model, request).await
    }
}

#[derive(Clone)]
pub struct OpenAICompatibleSpeech {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAICompatibleSpeech {
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

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_COMPAT_API_KEY",
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
        ];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_optional(config, env, DEFAULT_KEYS)
                .await?,
        })
    }

    fn resolve_model<'a>(&'a self, request: &'a SpeechRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.client.model.trim().is_empty() {
            return Ok(self.client.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai-compatible speech model is not set (set request.model or OpenAICompatibleSpeech::with_model)"
                .to_string(),
        ))
    }
}

#[async_trait]
impl SpeechModel for OpenAICompatibleSpeech {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn speak(&self, request: SpeechRequest) -> Result<SpeechResponse> {
        let model = self.resolve_model(&request)?.to_string();
        openai_audio_common::speak(self.provider(), &self.client, model, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SpeechResponseFormat, TranscriptionResponseFormat, Warning};
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn transcribe_posts_multipart_and_parses_json() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/audio/transcriptions")
                    .body_includes("name=\"model\"")
                    .body_includes("whisper-1")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"text\":\"ok\"}");
            })
            .await;

        let client = OpenAICompatibleAudioTranscription::new("")
            .with_base_url(server.url("/v1"))
            .with_model("whisper-1");
        let response = client
            .transcribe(AudioTranscriptionRequest {
                audio: b"hello".to_vec(),
                filename: "audio.wav".to_string(),
                media_type: Some("audio/wav".to_string()),
                model: None,
                language: None,
                prompt: None,
                response_format: Some(TranscriptionResponseFormat::Json),
                temperature: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.text, "ok");
        Ok(())
    }

    #[tokio::test]
    async fn translate_posts_multipart_and_parses_json() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/audio/translations")
                    .body_includes("name=\"model\"")
                    .body_includes("whisper-1")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{\"text\":\"ok\"}");
            })
            .await;

        let client = OpenAICompatibleAudioTranscription::new("")
            .with_base_url(server.url("/v1"))
            .with_model("whisper-1");
        let response = client
            .translate(AudioTranscriptionRequest {
                audio: b"hello".to_vec(),
                filename: "audio.wav".to_string(),
                media_type: Some("audio/wav".to_string()),
                model: None,
                language: None,
                prompt: None,
                response_format: Some(TranscriptionResponseFormat::Json),
                temperature: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.text, "ok");
        Ok(())
    }

    #[tokio::test]
    async fn transcribe_json_parse_falls_back_to_text_with_warning() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/audio/transcriptions")
                    .body_includes("name=\"model\"")
                    .body_includes("whisper-1")
                    .body_includes("name=\"file\"")
                    .body_includes("hello");
                then.status(200)
                    .header("content-type", "text/plain")
                    .body("not json");
            })
            .await;

        let client = OpenAICompatibleAudioTranscription::new("")
            .with_base_url(server.url("/v1"))
            .with_model("whisper-1");
        let response = client
            .transcribe(AudioTranscriptionRequest {
                audio: b"hello".to_vec(),
                filename: "audio.wav".to_string(),
                media_type: Some("audio/wav".to_string()),
                model: None,
                language: None,
                prompt: None,
                response_format: Some(TranscriptionResponseFormat::Json),
                temperature: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.text, "not json");
        assert!(response.warnings.iter().any(|warning| matches!(
            warning,
            Warning::Compatibility { feature, .. } if feature == "audio.transcription.json"
        )));
        Ok(())
    }

    #[tokio::test]
    async fn speak_posts_json_and_returns_audio() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/audio/speech")
                    .body_includes("\"model\":\"gpt-4o-mini-tts\"")
                    .body_includes("\"voice\":\"alloy\"")
                    .body_includes("\"input\":\"hi\"");
                then.status(200)
                    .header("content-type", "audio/mpeg")
                    .body("MP3DATA");
            })
            .await;

        let client = OpenAICompatibleSpeech::new("")
            .with_base_url(server.url("/v1"))
            .with_model("gpt-4o-mini-tts");
        let response = client
            .speak(SpeechRequest {
                input: "hi".to_string(),
                voice: "alloy".to_string(),
                model: None,
                response_format: Some(SpeechResponseFormat::Mp3),
                speed: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.audio, b"MP3DATA".to_vec());
        assert_eq!(response.media_type.as_deref(), Some("audio/mpeg"));
        Ok(())
    }
}
