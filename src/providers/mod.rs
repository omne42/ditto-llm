#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "bedrock")]
pub mod bedrock;
#[cfg(feature = "cohere")]
pub mod cohere;
#[cfg(any(feature = "google", feature = "vertex"))]
mod genai;
#[cfg(feature = "google")]
pub mod google;
#[cfg(any(feature = "openai", feature = "openai-compatible"))]
pub mod openai;
#[cfg(feature = "openai-compatible")]
pub mod openai_compatible;
#[cfg(all(feature = "audio", feature = "openai-compatible"))]
pub mod openai_compatible_audio;
#[cfg(all(feature = "batches", feature = "openai-compatible"))]
pub mod openai_compatible_batches;
pub mod openai_compatible_family;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub mod openai_compatible_images;
#[cfg(all(feature = "moderations", feature = "openai-compatible"))]
pub mod openai_compatible_moderations;
#[cfg(feature = "vertex")]
pub mod vertex;

#[cfg(all(
    feature = "audio",
    any(feature = "openai", feature = "openai-compatible")
))]
mod openai_audio_common;
#[cfg(all(
    feature = "batches",
    any(feature = "openai", feature = "openai-compatible")
))]
mod openai_batches_common;
#[cfg(all(
    feature = "embeddings",
    any(feature = "openai", feature = "openai-compatible")
))]
mod openai_embeddings_common;
#[cfg(all(
    feature = "images",
    any(feature = "openai", feature = "openai-compatible")
))]
mod openai_images_common;
#[cfg(any(feature = "openai", feature = "openai-compatible"))]
mod openai_like;
#[cfg(all(
    feature = "moderations",
    any(feature = "openai", feature = "openai-compatible")
))]
mod openai_moderations_common;
#[cfg(all(feature = "videos", feature = "openai"))]
mod openai_videos_common;

#[cfg(feature = "anthropic")]
pub use anthropic::Anthropic;
#[cfg(feature = "bedrock")]
pub use bedrock::Bedrock;
#[cfg(feature = "cohere")]
pub use cohere::Cohere;
#[cfg(all(feature = "cohere", feature = "embeddings"))]
pub use cohere::CohereEmbeddings;
#[cfg(all(feature = "cohere", feature = "rerank"))]
pub use cohere::CohereRerank;
#[cfg(feature = "google")]
pub use google::Google;
#[cfg(all(feature = "google", feature = "embeddings"))]
pub use google::GoogleEmbeddings;
#[cfg(all(feature = "google", feature = "images"))]
pub use google::GoogleImages;
#[cfg(all(feature = "google", feature = "realtime"))]
pub use google::GoogleRealtime;
#[cfg(all(feature = "google", feature = "videos"))]
pub use google::GoogleVideos;
#[cfg(feature = "openai")]
pub use openai::OpenAI;
#[cfg(all(feature = "batches", feature = "openai"))]
pub use openai::OpenAIBatches;
#[cfg(all(feature = "openai", feature = "openai-compatible"))]
pub use openai::OpenAIChatCompletions;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub use openai::OpenAICompatibleImageEdits;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use openai::OpenAIEmbeddings;
#[cfg(all(feature = "images", feature = "openai"))]
pub use openai::OpenAIImageEdits;
#[cfg(all(feature = "images", feature = "openai"))]
pub use openai::OpenAIImages;
#[cfg(all(feature = "moderations", feature = "openai"))]
pub use openai::OpenAIModerations;
#[cfg(all(feature = "realtime", feature = "openai"))]
pub use openai::OpenAIRealtime;
#[cfg(all(feature = "videos", feature = "openai"))]
pub use openai::OpenAIVideos;
#[cfg(all(feature = "audio", feature = "openai"))]
pub use openai::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(feature = "openai")]
pub use openai::{OpenAICompletionsLegacy, OpenAITextModel};
#[cfg(feature = "openai-compatible")]
pub use openai_compatible::OpenAICompatible;
#[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
pub use openai_compatible::OpenAICompatibleEmbeddings;
#[cfg(all(feature = "audio", feature = "openai-compatible"))]
pub use openai_compatible_audio::{OpenAICompatibleAudioTranscription, OpenAICompatibleSpeech};
#[cfg(all(feature = "batches", feature = "openai-compatible"))]
pub use openai_compatible_batches::OpenAICompatibleBatches;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub use openai_compatible_images::OpenAICompatibleImages;
#[cfg(all(feature = "moderations", feature = "openai-compatible"))]
pub use openai_compatible_moderations::OpenAICompatibleModerations;
#[cfg(feature = "vertex")]
pub use vertex::Vertex;
