use async_trait::async_trait;

use super::openai_audio_common;
use super::openai_like;

use crate::audio::{AudioTranscriptionModel, AudioTranslationModel, SpeechModel};
use crate::profile::{Env, ProviderConfig};
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
};
use crate::{DittoError, Result};

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

            fn resolve_model<'a>(
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

        #[async_trait]
        impl AudioTranslationModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn translate(
                &self,
                request: AudioTranscriptionRequest,
            ) -> Result<AudioTranscriptionResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_audio_common::translate($provider, &self.client, model, request).await
            }
        }
    };
}

macro_rules! define_openai_like_speech {
    (
        $name:ident,
        provider = $provider:literal,
        default_keys = $default_keys:expr,
        from_config = $from_config:path,
        missing_model_error = $missing_model_error:literal $(,)?
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
                Err(DittoError::InvalidResponse(
                    $missing_model_error.to_string(),
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

#[cfg(feature = "openai")]
define_openai_like_audio_transcription!(
    OpenAIAudioTranscription,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error = "openai audio transcription model is not set (set request.model or OpenAIAudioTranscription::with_model)",
);

#[cfg(feature = "openai")]
define_openai_like_speech!(
    OpenAISpeech,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error =
        "openai speech model is not set (set request.model or OpenAISpeech::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_audio_transcription!(
    OpenAICompatibleAudioTranscription,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY",],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible audio transcription model is not set (set request.model or OpenAICompatibleAudioTranscription::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_speech!(
    OpenAICompatibleSpeech,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY",],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible speech model is not set (set request.model or OpenAICompatibleSpeech::with_model)",
);

#[cfg(test)]
mod tests {
    use crate::types::{SpeechResponseFormat, TranscriptionResponseFormat, Warning};
    use httpmock::{Method::POST, MockServer};

    #[cfg(feature = "openai")]
    mod openai {
        use super::super::*;
        use super::*;

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

            let client = OpenAIAudioTranscription::new("")
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
        async fn transcribe_merges_provider_options_into_multipart_form() -> Result<()> {
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
                        .body_includes("hello")
                        .body_includes("name=\"extra\"")
                        .body_includes("ok")
                        .body_includes("name=\"tags\"")
                        .body_includes("tag-a")
                        .body_includes("tag-b")
                        .body_excludes("evil-model");
                    then.status(200)
                        .header("content-type", "application/json")
                        .body("{\"text\":\"ok\"}");
                })
                .await;

            let client = OpenAIAudioTranscription::new("")
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
                    provider_options: Some(serde_json::json!({
                        "extra": "ok",
                        "tags": ["tag-a", "tag-b"],
                        "model": "evil-model"
                    })),
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

            let client = OpenAIAudioTranscription::new("")
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

            let client = OpenAIAudioTranscription::new("")
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

            let client = OpenAISpeech::new("")
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

        #[tokio::test]
        async fn speak_is_bounded() -> Result<()> {
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

            let client = OpenAISpeech::new("")
                .with_base_url(server.url("/v1"))
                .with_model("gpt-4o-mini-tts")
                .with_max_binary_response_bytes(4);
            let err = client
                .speak(SpeechRequest {
                    input: "hi".to_string(),
                    voice: "alloy".to_string(),
                    model: None,
                    response_format: Some(SpeechResponseFormat::Mp3),
                    speed: None,
                    provider_options: None,
                })
                .await
                .unwrap_err();

            mock.assert_async().await;
            match err {
                DittoError::InvalidResponse(message) => {
                    assert!(message.contains("exceeds max bytes"));
                }
                other => panic!("unexpected error: {other:?}"),
            }
            Ok(())
        }
    }

    #[cfg(feature = "openai-compatible")]
    mod openai_compatible {
        use super::super::*;
        use super::*;

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
}
