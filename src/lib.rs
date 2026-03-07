pub mod audio;
pub mod batch;
pub mod catalog;
mod error;
pub mod file;
pub mod image;
pub mod moderation;
pub mod object;
mod profile;
pub mod rerank;
pub mod secrets;
mod stream;
pub mod text;

pub mod embedding;
pub mod layer;
pub mod model;
pub mod providers;
pub mod types;
pub mod utils;

#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "auth")]
pub mod auth;
#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "sdk")]
pub mod sdk;

pub use error::{DittoError, Result};
pub use profile::{
    AnthropicCatalogProvider, AnthropicModelCatalog, AnthropicModelCatalogEntry,
    AnthropicModelPricing, AnthropicModelStatus, BuiltinProviderModelCandidate,
    BuiltinProviderPreset, ConfigScope, Env, GoogleCatalogProvider, GoogleModelCatalog,
    GoogleModelCatalogEntry, GoogleModelVersion, GoogleSupportedDataTypes, ModelConfig,
    ModelDeleteReport, ModelDeleteRequest, ModelListReport, ModelListRequest, ModelShowReport,
    ModelShowRequest, ModelSummary, ModelUpsertReport, ModelUpsertRequest, OpenAiCatalogProvider,
    OpenAiCompatibleClient, OpenAiModalitySupport, OpenAiModelCatalog, OpenAiModelCatalogEntry,
    OpenAiModelRevisions, OpenAiModelsProvider, OpenAiProviderFamily, OpenAiProviderQuirks,
    PromptCacheUsageReporting, Provider, ProviderApi, ProviderAuth, ProviderAuthType,
    ProviderCapabilities, ProviderConfig, ProviderDeleteReport, ProviderDeleteRequest,
    ProviderListReport, ProviderListRequest, ProviderNamespace, ProviderRoutingConfig,
    ProviderShowReport, ProviderShowRequest, ProviderSummary, ProviderUpsertReport,
    ProviderUpsertRequest, ResolvedRoutingPlan, ResolvedRoutingTarget, RoutingConfigFormat,
    RoutingContext, RoutingOverride, RoutingPhase, RoutingPolicy, RoutingPolicySource,
    RoutingProviderProfile, RoutingStagePolicy, RoutingTarget, ThinkingIntensity,
    anthropic_model_catalog, anthropic_model_catalog_entry, anthropic_model_catalog_entry_by_model,
    builtin_models_for_provider, builtin_provider_candidates_for_model, builtin_provider_preset,
    builtin_provider_presets, complete_model_upsert_request_interactive,
    complete_provider_upsert_request_interactive, delete_model_config, delete_provider_config,
    filter_models_whitelist, google_model_catalog, google_model_catalog_entry,
    google_model_catalog_entry_by_model, infer_openai_provider_family,
    infer_openai_provider_quirks, list_available_models, list_model_configs, list_provider_configs,
    merge_provider_config, normalize_string_list, openai_model_catalog, openai_model_catalog_entry,
    parse_dotenv, resolve_auth_token, resolve_auth_token_with_default_keys, select_model_config,
    show_model_config, show_provider_config, upsert_model_config, upsert_provider_config,
};

pub use audio::{AudioTranscriptionModel, AudioTranslationModel, SpeechModel};
pub use batch::BatchClient;
pub use catalog::{
    ApiSurfaceId, AuthMethodKind, CatalogRegistry, EndpointQueryParam, EndpointTemplate,
    EvidenceLevel, EvidenceRef, HttpMethod, InvocationHints, ModelBinding, ModelSelector,
    OperationKind, ProtocolQuirks, ProviderAuthHint, ProviderClass, ProviderModelDescriptor,
    ProviderPluginDescriptor, ResolvedEndpoint, ResolvedInvocation, RuntimeRoute,
    RuntimeRouteRequest, TransportKind, VerificationStatus, WireProtocol, builtin_provider_plugins,
    builtin_registry,
};
pub use embedding::{EmbeddingModel, EmbeddingModelExt};
pub use file::{FileClient, FileContent, FileDeleteResponse, FileObject, FileUploadRequest};
pub use image::ImageGenerationModel;
pub use layer::{LanguageModelLayer, LanguageModelLayerExt, LayeredLanguageModel};
pub use model::{LanguageModel, StreamResult};
pub use moderation::ModerationModel;
pub use object::{
    GenerateObjectResponse, LanguageModelObjectExt, ObjectOptions, ObjectOutput, ObjectStrategy,
    StreamObjectFinal, StreamObjectHandle, StreamObjectResult,
};
pub use rerank::RerankModel;
pub use stream::{
    AbortableStream, CollectedStream, LanguageModelExt, StreamAbortHandle, abortable_stream,
    collect_stream,
};
pub use text::{
    GenerateTextResponse, LanguageModelTextExt, StreamTextFinal, StreamTextHandle, StreamTextResult,
};

#[cfg(feature = "sdk")]
pub use sdk::cache::CacheLayer;
pub use types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchRequestCounts, BatchResponse, BatchStatus, ContentPart, FileSource,
    FinishReason, GenerateRequest, GenerateResponse, ImageGenerationRequest,
    ImageGenerationResponse, ImageResponseFormat, ImageSource, JsonSchemaFormat, Message,
    ModerationInput, ModerationRequest, ModerationResponse, ModerationResult, ProviderOptions,
    ReasoningEffort, ReasoningSummary, RerankDocument, RerankRequest, RerankResponse, RerankResult,
    ResponseFormat, Role, SpeechRequest, SpeechResponse, SpeechResponseFormat, StreamChunk, Tool,
    ToolChoice, TranscriptionResponseFormat, Usage, Warning,
};

#[cfg(feature = "anthropic")]
pub use providers::Anthropic;
#[cfg(feature = "bedrock")]
pub use providers::Bedrock;
#[cfg(feature = "cohere")]
pub use providers::Cohere;
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
#[cfg(feature = "vertex")]
pub use providers::Vertex;
#[cfg(all(feature = "openai", feature = "audio"))]
pub use providers::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(all(feature = "openai-compatible", feature = "audio"))]
pub use providers::{OpenAICompatibleAudioTranscription, OpenAICompatibleSpeech};
