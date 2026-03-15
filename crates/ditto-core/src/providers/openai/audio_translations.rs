use async_trait::async_trait;

use crate::capabilities::audio::AudioTranslationModel;
use crate::error::Result;
use crate::types::{AudioTranscriptionRequest, AudioTranscriptionResponse};

#[cfg(feature = "provider-openai")]
use super::audio_transcriptions::OpenAIAudioTranscription;
#[cfg(feature = "provider-openai-compatible")]
use super::audio_transcriptions::OpenAICompatibleAudioTranscription;

#[cfg(feature = "provider-openai")]
#[async_trait]
impl AudioTranslationModel for OpenAIAudioTranscription {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn translate(
        &self,
        request: AudioTranscriptionRequest,
    ) -> Result<AudioTranscriptionResponse> {
        let model = self.resolve_model(&request)?.to_string();
        crate::providers::openai_audio_common::translate(
            self.provider(),
            &self.client,
            model,
            request,
        )
        .await
    }
}

#[cfg(feature = "provider-openai-compatible")]
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
        crate::providers::openai_audio_common::translate(
            self.provider(),
            &self.client,
            model,
            request,
        )
        .await
    }
}
