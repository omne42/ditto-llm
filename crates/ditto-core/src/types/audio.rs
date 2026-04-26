use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::Warning;
use crate::provider_options::ProviderOptionsEnvelope;
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

impl TranscriptionResponseFormat {
    #[must_use]
    pub fn to_foundation(self) -> speech_transcription_kit::TranscriptionResponseFormat {
        match self {
            Self::Json => speech_transcription_kit::TranscriptionResponseFormat::Json,
            Self::Text => speech_transcription_kit::TranscriptionResponseFormat::Text,
            Self::Srt => speech_transcription_kit::TranscriptionResponseFormat::Srt,
            Self::VerboseJson => speech_transcription_kit::TranscriptionResponseFormat::VerboseJson,
            Self::Vtt => speech_transcription_kit::TranscriptionResponseFormat::Vtt,
        }
    }
}

impl From<speech_transcription_kit::TranscriptionResponseFormat> for TranscriptionResponseFormat {
    fn from(value: speech_transcription_kit::TranscriptionResponseFormat) -> Self {
        match value {
            speech_transcription_kit::TranscriptionResponseFormat::Json => Self::Json,
            speech_transcription_kit::TranscriptionResponseFormat::Text => Self::Text,
            speech_transcription_kit::TranscriptionResponseFormat::Srt => Self::Srt,
            speech_transcription_kit::TranscriptionResponseFormat::VerboseJson => Self::VerboseJson,
            speech_transcription_kit::TranscriptionResponseFormat::Vtt => Self::Vtt,
        }
    }
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
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

impl AudioTranscriptionRequest {
    #[must_use]
    pub fn to_foundation(&self) -> speech_transcription_kit::TranscriptionRequest {
        speech_transcription_kit::TranscriptionRequest {
            audio: speech_transcription_kit::TranscriptionAudioSource::InlineBytes {
                data: self.audio.clone(),
                file_name: self.filename.clone(),
                media_type: self.media_type.clone(),
            },
            options: speech_transcription_kit::TranscriptionOptions {
                model: self.model.clone(),
                language: self.language.clone(),
                prompt: self.prompt.clone(),
                response_format: self
                    .response_format
                    .map(TranscriptionResponseFormat::to_foundation),
                temperature: self.temperature,
            },
        }
    }
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

impl From<speech_transcription_kit::TranscriptionResponse> for AudioTranscriptionResponse {
    fn from(value: speech_transcription_kit::TranscriptionResponse) -> Self {
        Self {
            text: value.text,
            warnings: Vec::new(),
            provider_metadata: value.provider_metadata,
        }
    }
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
    pub provider_options: Option<ProviderOptionsEnvelope>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_request_converts_to_foundation_dto() {
        let request = AudioTranscriptionRequest {
            audio: vec![1, 2, 3],
            filename: "sample.wav".to_string(),
            media_type: Some("audio/wav".to_string()),
            model: Some("whisper-1".to_string()),
            language: Some("en".to_string()),
            prompt: Some("terms".to_string()),
            response_format: Some(TranscriptionResponseFormat::VerboseJson),
            temperature: Some(0.2),
            provider_options: None,
        };

        let foundation = request.to_foundation();
        let speech_transcription_kit::TranscriptionAudioSource::InlineBytes {
            data,
            file_name,
            media_type,
        } = foundation.audio;
        assert_eq!(data, vec![1, 2, 3]);
        assert_eq!(file_name, "sample.wav");
        assert_eq!(media_type.as_deref(), Some("audio/wav"));
        assert_eq!(foundation.options.model.as_deref(), Some("whisper-1"));
        assert_eq!(
            foundation.options.response_format,
            Some(speech_transcription_kit::TranscriptionResponseFormat::VerboseJson)
        );
        assert_eq!(foundation.options.temperature, Some(0.2));
    }

    #[test]
    fn transcription_response_converts_from_foundation_dto() {
        let response =
            AudioTranscriptionResponse::from(speech_transcription_kit::TranscriptionResponse {
                text: "hello".to_string(),
                provider_metadata: Some(serde_json::json!({ "provider": "test" })),
            });

        assert_eq!(response.text, "hello");
        assert!(response.warnings.is_empty());
        assert_eq!(
            response.provider_metadata,
            Some(serde_json::json!({ "provider": "test" }))
        );
    }
}
