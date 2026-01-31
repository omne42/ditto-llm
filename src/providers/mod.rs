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
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(all(feature = "audio", feature = "openai"))]
pub mod openai_audio;
#[cfg(all(feature = "batches", feature = "openai"))]
pub mod openai_batches;
#[cfg(feature = "openai-compatible")]
pub mod openai_compatible;
#[cfg(all(feature = "audio", feature = "openai-compatible"))]
pub mod openai_compatible_audio;
#[cfg(all(feature = "batches", feature = "openai-compatible"))]
pub mod openai_compatible_batches;
#[cfg(all(feature = "images", feature = "openai-compatible"))]
pub mod openai_compatible_images;
#[cfg(all(feature = "moderations", feature = "openai-compatible"))]
pub mod openai_compatible_moderations;
#[cfg(all(feature = "images", feature = "openai"))]
pub mod openai_images;
#[cfg(all(feature = "moderations", feature = "openai"))]
pub mod openai_moderations;
#[cfg(feature = "vertex")]
pub mod vertex;

#[cfg(feature = "openai")]
mod openai_like;

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
#[cfg(feature = "openai")]
pub use openai::OpenAI;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use openai::OpenAIEmbeddings;
#[cfg(all(feature = "audio", feature = "openai"))]
pub use openai_audio::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(all(feature = "batches", feature = "openai"))]
pub use openai_batches::OpenAIBatches;
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
#[cfg(all(feature = "images", feature = "openai"))]
pub use openai_images::OpenAIImages;
#[cfg(all(feature = "moderations", feature = "openai"))]
pub use openai_moderations::OpenAIModerations;
#[cfg(feature = "vertex")]
pub use vertex::Vertex;
