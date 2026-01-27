use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::audio::{AudioTranscriptionModel, SpeechModel};
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
    SpeechResponseFormat, TranscriptionResponseFormat, Warning,
};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAIAudioTranscription {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAIAudioTranscription {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            auth,
            model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn transcriptions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/audio/transcriptions") {
            base.to_string()
        } else {
            format!("{base}/audio/transcriptions")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a AudioTranscriptionRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai audio transcription model is not set (set request.model or OpenAIAudioTranscription::with_model)"
                .to_string(),
        ))
    }

    fn transcription_format_to_str(format: TranscriptionResponseFormat) -> &'static str {
        match format {
            TranscriptionResponseFormat::Json => "json",
            TranscriptionResponseFormat::Text => "text",
            TranscriptionResponseFormat::Srt => "srt",
            TranscriptionResponseFormat::VerboseJson => "verbose_json",
            TranscriptionResponseFormat::Vtt => "vtt",
        }
    }

    fn merge_provider_options(
        _form: Form,
        options: Option<&Value>,
        warnings: &mut Vec<Warning>,
    ) -> Form {
        let Some(options) = options else {
            return _form;
        };
        warnings.push(Warning::Unsupported {
            feature: "audio.provider_options".to_string(),
            details: Some(format!(
                "provider_options are not supported for audio multipart requests; got {options:?}"
            )),
        });
        _form
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptionJsonResponse {
    #[serde(default)]
    text: String,
}

#[async_trait]
impl AudioTranscriptionModel for OpenAIAudioTranscription {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn transcribe(
        &self,
        request: AudioTranscriptionRequest,
    ) -> Result<AudioTranscriptionResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let mut warnings = Vec::<Warning>::new();

        let mut file_part = Part::bytes(request.audio).file_name(request.filename);
        if let Some(media_type) = request
            .media_type
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            file_part = file_part.mime_str(media_type).map_err(|err| {
                DittoError::InvalidResponse(format!("invalid transcription media type: {err}"))
            })?;
        }

        let mut form = Form::new()
            .text("model", model.clone())
            .part("file", file_part);
        if let Some(language) = request.language.as_deref().filter(|s| !s.trim().is_empty()) {
            form = form.text("language", language.to_string());
        }
        if let Some(prompt) = request.prompt.as_deref().filter(|s| !s.trim().is_empty()) {
            form = form.text("prompt", prompt.to_string());
        }
        if let Some(format) = request.response_format {
            form = form.text("response_format", Self::transcription_format_to_str(format));
        }
        if let Some(temperature) = request.temperature {
            if temperature.is_finite() {
                form = form.text("temperature", temperature.to_string());
            } else {
                warnings.push(Warning::Compatibility {
                    feature: "temperature".to_string(),
                    details: format!("temperature is not finite ({temperature}); dropping"),
                });
            }
        }

        form =
            Self::merge_provider_options(form, selected_provider_options.as_ref(), &mut warnings);

        let url = self.transcriptions_url();
        let response = self
            .apply_auth(self.http.post(url))
            .multipart(form)
            .send()
            .await?;

        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&body).to_string();
            return Err(DittoError::Api { status, body: text });
        }

        let format = request
            .response_format
            .unwrap_or(TranscriptionResponseFormat::Json);
        let text = match format {
            TranscriptionResponseFormat::Text
            | TranscriptionResponseFormat::Srt
            | TranscriptionResponseFormat::Vtt => String::from_utf8_lossy(&body).to_string(),
            TranscriptionResponseFormat::Json | TranscriptionResponseFormat::VerboseJson => {
                match serde_json::from_slice::<TranscriptionJsonResponse>(&body) {
                    Ok(parsed) => parsed.text,
                    Err(err) => {
                        warnings.push(Warning::Compatibility {
                            feature: "audio.transcription.json".to_string(),
                            details: format!(
                                "failed to parse transcription JSON response; falling back to text: {err}"
                            ),
                        });
                        String::from_utf8_lossy(&body).to_string()
                    }
                }
            }
        };

        Ok(AudioTranscriptionResponse {
            text,
            warnings,
            provider_metadata: Some(
                serde_json::json!({ "model": model, "response_format": Self::transcription_format_to_str(format) }),
            ),
        })
    }
}

#[derive(Clone)]
pub struct OpenAISpeech {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAISpeech {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
            auth,
            model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn speech_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/audio/speech") {
            base.to_string()
        } else {
            format!("{base}/audio/speech")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a SpeechRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai speech model is not set (set request.model or OpenAISpeech::with_model)"
                .to_string(),
        ))
    }

    fn speech_format_to_str(format: SpeechResponseFormat) -> &'static str {
        match format {
            SpeechResponseFormat::Mp3 => "mp3",
            SpeechResponseFormat::Opus => "opus",
            SpeechResponseFormat::Aac => "aac",
            SpeechResponseFormat::Flac => "flac",
            SpeechResponseFormat::Wav => "wav",
            SpeechResponseFormat::Pcm => "pcm",
        }
    }

    fn merge_provider_options(
        body: &mut Map<String, Value>,
        options: Option<&Value>,
        warnings: &mut Vec<Warning>,
    ) {
        let Some(options) = options else {
            return;
        };
        let Some(obj) = options.as_object() else {
            warnings.push(Warning::Unsupported {
                feature: "audio.provider_options".to_string(),
                details: Some("expected provider_options to be a JSON object".to_string()),
            });
            return;
        };

        for (key, value) in obj {
            if body.contains_key(key) {
                warnings.push(Warning::Compatibility {
                    feature: "audio.provider_options".to_string(),
                    details: format!("provider_options overrides {key}; ignoring override"),
                });
                continue;
            }
            body.insert(key.clone(), value.clone());
        }
    }
}

#[async_trait]
impl SpeechModel for OpenAISpeech {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn speak(&self, request: SpeechRequest) -> Result<SpeechResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let mut warnings = Vec::<Warning>::new();

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.clone()));
        body.insert("input".to_string(), Value::String(request.input));
        body.insert("voice".to_string(), Value::String(request.voice));
        if let Some(format) = request.response_format {
            body.insert(
                "response_format".to_string(),
                Value::String(Self::speech_format_to_str(format).to_string()),
            );
        }
        if let Some(speed) = request.speed {
            if speed.is_finite() {
                body.insert(
                    "speed".to_string(),
                    Value::Number(
                        serde_json::Number::from_f64(speed as f64).unwrap_or_else(|| 1.into()),
                    ),
                );
            } else {
                warnings.push(Warning::Compatibility {
                    feature: "speed".to_string(),
                    details: format!("speed is not finite ({speed}); dropping"),
                });
            }
        }

        Self::merge_provider_options(&mut body, selected_provider_options.as_ref(), &mut warnings);

        let url = self.speech_url();
        let response = self
            .apply_auth(self.http.post(url))
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        let bytes = response.bytes().await?;

        Ok(SpeechResponse {
            audio: bytes.to_vec(),
            media_type,
            warnings,
            provider_metadata: Some(serde_json::json!({ "model": model })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
