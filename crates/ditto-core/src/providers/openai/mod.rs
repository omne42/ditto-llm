#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod audio_speech;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod audio_transcriptions;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod audio_translations;
#[cfg(all(feature = "cap-batch", feature = "provider-openai"))]
mod batches;
#[cfg(feature = "provider-openai")]
mod chat_completions;
mod client;
#[cfg(all(feature = "provider-openai", feature = "cap-embedding"))]
mod embeddings;
mod files;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod images_edits;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod images_generations;
mod models;
#[cfg(all(
    feature = "cap-moderation",
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod moderations;
mod raw_responses;
#[cfg(all(feature = "cap-realtime", feature = "provider-openai"))]
mod realtime;
#[cfg(feature = "provider-openai")]
mod responses;
#[cfg(feature = "provider-openai")]
mod text;
#[cfg(all(feature = "cap-video-generation", feature = "provider-openai"))]
mod videos;

#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai-compatible"
))]
pub use audio_speech::OpenAICompatibleSpeech;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai"
))]
pub use audio_speech::OpenAISpeech;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai"
))]
pub use audio_transcriptions::OpenAIAudioTranscription;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai-compatible"
))]
pub use audio_transcriptions::OpenAICompatibleAudioTranscription;
#[cfg(all(feature = "cap-batch", feature = "provider-openai"))]
pub use batches::OpenAIBatches;
#[cfg(feature = "provider-openai")]
pub use chat_completions::OpenAIChatCompletions;
pub use client::OpenAI;
#[cfg(all(feature = "provider-openai", feature = "cap-embedding"))]
pub use embeddings::OpenAIEmbeddings;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai-compatible"
))]
pub use images_edits::OpenAICompatibleImageEdits;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai"
))]
pub use images_edits::OpenAIImageEdits;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai-compatible"
))]
pub use images_generations::OpenAICompatibleImages;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai"
))]
pub use images_generations::OpenAIImages;
pub use models::{OpenAIModelObject, OpenAIModelPermission};
#[cfg(all(feature = "cap-moderation", feature = "provider-openai-compatible"))]
pub use moderations::OpenAICompatibleModerations;
#[cfg(all(feature = "cap-moderation", feature = "provider-openai"))]
pub use moderations::OpenAIModerations;
pub use raw_responses::{
    OpenAIResponsesCompactionRequest, OpenAIResponsesRawEvent, OpenAIResponsesRawEventStream,
    OpenAIResponsesRawRequest,
};
#[cfg(all(feature = "cap-realtime", feature = "provider-openai"))]
pub use realtime::OpenAIRealtime;
#[cfg(feature = "provider-openai")]
pub use text::OpenAITextModel;
#[cfg(all(feature = "cap-video-generation", feature = "provider-openai"))]
pub use videos::OpenAIVideos;
