pub mod audio;
pub mod batch;
mod error;
pub mod image;
pub mod moderation;
pub mod object;
mod profile;
pub mod rerank;
mod stream;
pub mod text;

pub mod embedding;
pub mod model;
pub mod providers;
pub mod types;
pub mod utils;

pub use error::{DittoError, Result};
pub use profile::{
    Env, ModelConfig, OpenAiCompatibleClient, OpenAiProvider, Provider, ProviderAuth,
    ProviderCapabilities, ProviderConfig, ThinkingIntensity, filter_models_whitelist,
    list_available_models, normalize_string_list, parse_dotenv, resolve_auth_token,
    resolve_auth_token_with_default_keys, select_model_config,
};

pub use audio::{AudioTranscriptionModel, SpeechModel};
pub use batch::BatchClient;
pub use embedding::{EmbeddingModel, EmbeddingModelExt};
pub use image::ImageGenerationModel;
pub use model::{LanguageModel, StreamResult};
pub use moderation::ModerationModel;
pub use object::{
    GenerateObjectResponse, LanguageModelObjectExt, ObjectOptions, ObjectOutput, ObjectStrategy,
    StreamObjectFinal, StreamObjectResult,
};
pub use rerank::RerankModel;
pub use stream::{
    AbortableStream, CollectedStream, LanguageModelExt, StreamAbortHandle, abortable_stream,
    collect_stream,
};
pub use text::{GenerateTextResponse, LanguageModelTextExt, StreamTextFinal, StreamTextResult};
pub use types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchRequestCounts, BatchResponse, BatchStatus, ContentPart, FileSource,
    FinishReason, GenerateRequest, GenerateResponse, ImageGenerationRequest,
    ImageGenerationResponse, ImageResponseFormat, ImageSource, JsonSchemaFormat, Message,
    ModerationInput, ModerationRequest, ModerationResponse, ModerationResult, ProviderOptions,
    ReasoningEffort, RerankDocument, RerankRequest, RerankResponse, RerankResult, ResponseFormat,
    Role, SpeechRequest, SpeechResponse, SpeechResponseFormat, StreamChunk, Tool, ToolChoice,
    TranscriptionResponseFormat, Usage, Warning,
};

#[cfg(feature = "anthropic")]
pub use providers::Anthropic;
#[cfg(all(feature = "cohere", feature = "embeddings"))]
pub use providers::CohereEmbeddings;
#[cfg(all(feature = "cohere", feature = "rerank"))]
pub use providers::CohereRerank;
#[cfg(feature = "google")]
pub use providers::Google;
#[cfg(all(feature = "google", feature = "embeddings"))]
pub use providers::GoogleEmbeddings;
#[cfg(feature = "openai")]
pub use providers::OpenAI;
#[cfg(all(feature = "openai", feature = "batches"))]
pub use providers::OpenAIBatches;
#[cfg(feature = "openai-compatible")]
pub use providers::OpenAICompatible;
#[cfg(all(feature = "openai-compatible", feature = "batches"))]
pub use providers::OpenAICompatibleBatches;
#[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
pub use providers::OpenAICompatibleEmbeddings;
#[cfg(all(feature = "openai-compatible", feature = "images"))]
pub use providers::OpenAICompatibleImages;
#[cfg(all(feature = "openai-compatible", feature = "moderations"))]
pub use providers::OpenAICompatibleModerations;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use providers::OpenAIEmbeddings;
#[cfg(all(feature = "openai", feature = "images"))]
pub use providers::OpenAIImages;
#[cfg(all(feature = "openai", feature = "moderations"))]
pub use providers::OpenAIModerations;
#[cfg(all(feature = "openai", feature = "audio"))]
pub use providers::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(all(feature = "openai-compatible", feature = "audio"))]
pub use providers::{OpenAICompatibleAudioTranscription, OpenAICompatibleSpeech};
