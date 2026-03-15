#[cfg(feature = "provider-anthropic")]
pub mod anthropic;
#[cfg(feature = "provider-bedrock")]
pub mod bedrock;
#[cfg(feature = "provider-cohere")]
pub mod cohere;
#[cfg(any(feature = "provider-google", feature = "provider-vertex"))]
mod genai;
#[cfg(feature = "provider-google")]
pub mod google;
#[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
pub mod openai;
mod openai_compat_profile;
#[cfg(feature = "provider-openai-compatible")]
pub mod openai_compatible;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai-compatible"
))]
pub mod openai_compatible_audio;
#[cfg(all(feature = "cap-batch", feature = "provider-openai-compatible"))]
pub mod openai_compatible_batches;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai-compatible"
))]
pub mod openai_compatible_images;
#[cfg(all(feature = "cap-moderation", feature = "provider-openai-compatible"))]
pub mod openai_compatible_moderations;
#[cfg(feature = "provider-vertex")]
pub mod vertex;

#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod openai_audio_common;
#[cfg(all(
    feature = "cap-batch",
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod openai_batches_common;
#[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
mod openai_chat_completions_core;
#[cfg(all(
    feature = "cap-embedding",
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod openai_embeddings_common;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod openai_images_common;
#[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
mod openai_like;
#[cfg(all(
    feature = "cap-moderation",
    any(feature = "provider-openai", feature = "provider-openai-compatible")
))]
mod openai_moderations_common;
#[cfg(all(feature = "cap-video-generation", feature = "provider-openai"))]
mod openai_videos_common;

#[cfg(feature = "provider-anthropic")]
pub use anthropic::Anthropic;
#[cfg(feature = "provider-bedrock")]
pub use bedrock::Bedrock;
#[cfg(feature = "provider-cohere")]
pub use cohere::Cohere;
#[cfg(all(feature = "provider-cohere", feature = "cap-embedding"))]
pub use cohere::CohereEmbeddings;
#[cfg(all(feature = "provider-cohere", feature = "cap-rerank"))]
pub use cohere::CohereRerank;
#[cfg(feature = "provider-google")]
pub use google::Google;
#[cfg(all(feature = "provider-google", feature = "cap-embedding"))]
pub use google::GoogleEmbeddings;
#[cfg(all(
    feature = "provider-google",
    any(feature = "cap-image-generation", feature = "cap-image-edit")
))]
pub use google::GoogleImages;
#[cfg(all(feature = "provider-google", feature = "cap-realtime"))]
pub use google::GoogleRealtime;
#[cfg(all(feature = "provider-google", feature = "cap-video-generation"))]
pub use google::GoogleVideos;
#[cfg(feature = "provider-openai")]
pub use openai::OpenAI;
#[cfg(all(feature = "cap-batch", feature = "provider-openai"))]
pub use openai::OpenAIBatches;
#[cfg(feature = "provider-openai")]
pub use openai::OpenAIChatCompletions;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai-compatible"
))]
pub use openai::OpenAICompatibleImageEdits;
#[cfg(all(feature = "provider-openai", feature = "cap-embedding"))]
pub use openai::OpenAIEmbeddings;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai"
))]
pub use openai::OpenAIImageEdits;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai"
))]
pub use openai::OpenAIImages;
#[cfg(all(feature = "cap-moderation", feature = "provider-openai"))]
pub use openai::OpenAIModerations;
#[cfg(all(feature = "cap-realtime", feature = "provider-openai"))]
pub use openai::OpenAIRealtime;
#[cfg(feature = "provider-openai")]
pub use openai::OpenAITextModel;
#[cfg(all(feature = "cap-video-generation", feature = "provider-openai"))]
pub use openai::OpenAIVideos;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai"
))]
pub use openai::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(feature = "provider-openai-compatible")]
pub use openai_compatible::OpenAICompatible;
#[cfg(all(feature = "provider-openai-compatible", feature = "cap-embedding"))]
pub use openai_compatible::OpenAICompatibleEmbeddings;
#[cfg(all(
    any(feature = "cap-audio-transcription", feature = "cap-audio-speech"),
    feature = "provider-openai-compatible"
))]
pub use openai_compatible_audio::{OpenAICompatibleAudioTranscription, OpenAICompatibleSpeech};
#[cfg(all(feature = "cap-batch", feature = "provider-openai-compatible"))]
pub use openai_compatible_batches::OpenAICompatibleBatches;
#[cfg(all(
    any(feature = "cap-image-generation", feature = "cap-image-edit"),
    feature = "provider-openai-compatible"
))]
pub use openai_compatible_images::OpenAICompatibleImages;
#[cfg(all(feature = "cap-moderation", feature = "provider-openai-compatible"))]
pub use openai_compatible_moderations::OpenAICompatibleModerations;
#[cfg(feature = "provider-vertex")]
pub use vertex::Vertex;
