use async_trait::async_trait;

use crate::Result;
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
};

#[async_trait]
pub trait AudioTranscriptionModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn transcribe(
        &self,
        request: AudioTranscriptionRequest,
    ) -> Result<AudioTranscriptionResponse>;
}

#[async_trait]
pub trait SpeechModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn speak(&self, request: SpeechRequest) -> Result<SpeechResponse>;
}
