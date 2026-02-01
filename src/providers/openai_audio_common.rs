use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
    SpeechResponseFormat, TranscriptionResponseFormat, Warning,
};
use crate::{DittoError, Result};

#[derive(Debug, Deserialize)]
struct TranscriptionJsonResponse {
    #[serde(default)]
    text: String,
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

fn warn_unsupported_multipart_provider_options(
    options: Option<&Value>,
    warnings: &mut Vec<Warning>,
) {
    let Some(options) = options else {
        return;
    };
    warnings.push(Warning::Unsupported {
        feature: "audio.provider_options".to_string(),
        details: Some(format!(
            "provider_options are not supported for audio multipart requests; got {options:?}"
        )),
    });
}

pub(super) async fn transcribe(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: AudioTranscriptionRequest,
) -> Result<AudioTranscriptionResponse> {
    transcribe_to_endpoint(provider, client, model, request, "audio/transcriptions").await
}

pub(super) async fn translate(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: AudioTranscriptionRequest,
) -> Result<AudioTranscriptionResponse> {
    transcribe_to_endpoint(provider, client, model, request, "audio/translations").await
}

async fn transcribe_to_endpoint(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: AudioTranscriptionRequest,
    endpoint: &str,
) -> Result<AudioTranscriptionResponse> {
    let AudioTranscriptionRequest {
        audio,
        filename,
        media_type,
        model: _,
        language,
        prompt,
        response_format,
        temperature,
        provider_options,
    } = request;

    let selected_provider_options =
        crate::types::select_provider_options_value(provider_options.as_ref(), provider)?;
    let mut warnings = Vec::<Warning>::new();

    let mut file_part = Part::bytes(audio).file_name(filename);
    if let Some(media_type) = media_type.as_deref().filter(|s| !s.trim().is_empty()) {
        file_part = file_part.mime_str(media_type).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid transcription media type: {err}"))
        })?;
    }

    let mut form = Form::new()
        .text("model", model.clone())
        .part("file", file_part);
    if let Some(language) = language.as_deref().filter(|s| !s.trim().is_empty()) {
        form = form.text("language", language.to_string());
    }
    if let Some(prompt) = prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        form = form.text("prompt", prompt.to_string());
    }
    if let Some(format) = response_format {
        form = form.text("response_format", transcription_format_to_str(format));
    }
    if let Some(temperature) = temperature {
        if temperature.is_finite() {
            form = form.text("temperature", temperature.to_string());
        } else {
            warnings.push(Warning::Compatibility {
                feature: "temperature".to_string(),
                details: format!("temperature is not finite ({temperature}); dropping"),
            });
        }
    }

    warn_unsupported_multipart_provider_options(selected_provider_options.as_ref(), &mut warnings);

    let url = client.endpoint(endpoint);
    let response = client
        .apply_auth(client.http.post(url))
        .multipart(form)
        .send()
        .await?;

    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body).to_string();
        return Err(DittoError::Api { status, body: text });
    }

    let format = response_format.unwrap_or(TranscriptionResponseFormat::Json);
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
            serde_json::json!({ "model": model, "response_format": transcription_format_to_str(format) }),
        ),
    })
}

pub(super) async fn speak(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: SpeechRequest,
) -> Result<SpeechResponse> {
    let SpeechRequest {
        input,
        voice,
        model: _,
        response_format,
        speed,
        provider_options,
    } = request;

    let selected_provider_options =
        crate::types::select_provider_options_value(provider_options.as_ref(), provider)?;
    let mut warnings = Vec::<Warning>::new();

    let mut body = Map::<String, Value>::new();
    body.insert("model".to_string(), Value::String(model.clone()));
    body.insert("input".to_string(), Value::String(input));
    body.insert("voice".to_string(), Value::String(voice));
    if let Some(format) = response_format {
        body.insert(
            "response_format".to_string(),
            Value::String(speech_format_to_str(format).to_string()),
        );
    }
    if let Some(speed) = speed {
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

    crate::types::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &["model", "input", "voice", "response_format", "speed"],
        "audio.provider_options",
        &mut warnings,
    );

    let url = client.endpoint("audio/speech");
    let response = client
        .apply_auth(client.http.post(url))
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
