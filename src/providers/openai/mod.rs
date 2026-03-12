#[cfg(all(
    feature = "audio",
    any(feature = "openai", feature = "openai-compatible")
))]
mod audio_speech;
#[cfg(all(
    feature = "audio",
    any(feature = "openai", feature = "openai-compatible")
))]
mod audio_transcriptions;
#[cfg(all(
    feature = "audio",
    any(feature = "openai", feature = "openai-compatible")
))]
mod audio_translations;
#[cfg(all(feature = "batches", feature = "openai"))]
mod batches;
#[cfg(feature = "openai")]
mod chat_completions;
mod client;
#[cfg(feature = "openai")]
mod completions_legacy;
#[cfg(all(feature = "openai", feature = "embeddings"))]
mod embeddings;
mod files;
#[cfg(all(
    feature = "images",
    any(feature = "openai", feature = "openai-compatible")
))]
mod images_edits;
#[cfg(all(
    feature = "images",
    any(feature = "openai", feature = "openai-compatible")
))]
mod images_generations;
mod models;
#[cfg(all(
    feature = "moderations",
    any(feature = "openai", feature = "openai-compatible")
))]
mod moderations;
mod raw_responses;
#[cfg(all(feature = "realtime", feature = "openai"))]
mod realtime;
#[cfg(feature = "openai")]
mod responses;
#[cfg(feature = "openai")]
mod text;
#[cfg(all(feature = "videos", feature = "openai"))]
mod videos;

#[cfg(all(feature = "audio", feature = "openai-compatible"))]
pub use audio_speech::OpenAICompatibleSpeech;
#[cfg(all(feature = "audio", feature = "openai"))]
pub use audio_speech::OpenAISpeech;
#[cfg(all(feature = "audio", feature = "openai"))]
pub use audio_transcriptions::OpenAIAudioTranscription;
#[cfg(all(feature = "audio", feature = "openai-compatible"))]
pub use audio_transcriptions::OpenAICompatibleAudioTranscription;
#[cfg(all(feature = "batches", feature = "openai"))]
pub use batches::OpenAIBatches;
#[cfg(feature = "openai")]
pub use chat_completions::OpenAIChatCompletions;
pub use client::OpenAI;
#[cfg(feature = "openai")]
pub use completions_legacy::OpenAICompletionsLegacy;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use embeddings::OpenAIEmbeddings;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub use images_edits::OpenAICompatibleImageEdits;
#[cfg(all(feature = "images", feature = "openai"))]
pub use images_edits::OpenAIImageEdits;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub use images_generations::OpenAICompatibleImages;
#[cfg(all(feature = "images", feature = "openai"))]
pub use images_generations::OpenAIImages;
pub use models::{OpenAIModelObject, OpenAIModelPermission};
#[cfg(all(feature = "moderations", feature = "openai-compatible"))]
pub use moderations::OpenAICompatibleModerations;
#[cfg(all(feature = "moderations", feature = "openai"))]
pub use moderations::OpenAIModerations;
pub use raw_responses::{
    OpenAIResponsesCompactionRequest, OpenAIResponsesRawEvent, OpenAIResponsesRawEventStream,
    OpenAIResponsesRawRequest,
};
#[cfg(all(feature = "realtime", feature = "openai"))]
pub use realtime::OpenAIRealtime;
#[cfg(feature = "openai")]
pub use text::OpenAITextModel;
#[cfg(all(feature = "videos", feature = "openai"))]
pub use videos::OpenAIVideos;
