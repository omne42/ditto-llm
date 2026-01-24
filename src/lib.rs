mod error;
mod profile;
mod stream;

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

pub use embedding::EmbeddingModel;
pub use model::{LanguageModel, StreamResult};
pub use stream::{
    AbortableStream, CollectedStream, LanguageModelExt, StreamAbortHandle, abortable_stream,
    collect_stream,
};
pub use types::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource,
    JsonSchemaFormat, Message, ProviderOptions, ReasoningEffort, ResponseFormat, Role, StreamChunk,
    Tool, ToolChoice, Usage, Warning,
};

#[cfg(feature = "anthropic")]
pub use providers::Anthropic;
#[cfg(feature = "google")]
pub use providers::Google;
#[cfg(all(feature = "google", feature = "embeddings"))]
pub use providers::GoogleEmbeddings;
#[cfg(feature = "openai")]
pub use providers::OpenAI;
#[cfg(feature = "openai-compatible")]
pub use providers::OpenAICompatible;
#[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
pub use providers::OpenAICompatibleEmbeddings;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use providers::OpenAIEmbeddings;
