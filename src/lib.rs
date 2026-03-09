pub mod audio;
pub mod batch;
pub mod capabilities;
pub mod catalog;
pub mod config;
pub mod context_cache;
pub mod core;
mod error;
pub mod file;
pub mod image;
pub mod image_edit;
pub mod moderation;
pub mod object;
#[doc(hidden)]
pub mod profile;
pub mod realtime;
pub mod rerank;
pub mod runtime;
pub mod secrets;
mod stream;
pub mod text;
pub mod video;

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

pub use capabilities::{
    ContextCacheMode, ContextCacheModel, ContextCacheProfile, EmbeddingModel, EmbeddingModelExt,
    FileClient, FileContent, FileDeleteResponse, FileObject, FileUploadRequest,
    GenerateObjectResponse, GenerateTextResponse, ImageEditModel, ImageGenerationModel,
    LanguageModelObjectExt, LanguageModelTextExt, ModerationModel, ObjectOptions, ObjectOutput,
    ObjectStrategy, RealtimeSessionConnection, RealtimeSessionModel, RealtimeSessionRequest,
    RerankModel, SpeechModel, StreamObjectFinal, StreamObjectHandle, StreamObjectResult,
    StreamTextFinal, StreamTextHandle, StreamTextResult, VideoGenerationModel,
};
pub use catalog::{
    ApiSurfaceId, AssistantToolFollowupRequirement, AuthMethodKind, BehaviorSupport,
    CacheUsageReportingKind, CapabilityImplementationStatus, CapabilityKind,
    CapabilityStatusDescriptor, CatalogRegistry, ContextCacheModeId, EndpointQueryParam,
    EndpointTemplate, EvidenceLevel, EvidenceRef, HttpMethod, InvocationHints,
    ModelBehaviorDescriptor, ModelBinding, ModelCapabilityDescriptor, ModelSelector, OperationKind,
    ProtocolQuirks, ProviderAuthHint, ProviderCapabilityBinding, ProviderCapabilityResolution,
    ProviderCapabilitySet, ProviderClass, ProviderId, ProviderModelDescriptor,
    ProviderPluginDescriptor, ProviderProtocolFamily, ProviderRuntimeSpec, ReasoningActivationKind,
    ReasoningOutputMode, ReferenceCatalogExpectation, ReferenceCatalogExpectationIssue,
    ReferenceCatalogExpectationReport, ReferenceCatalogLoadError, ReferenceCatalogRole,
    ReferenceCatalogValidationIssue, ReferenceCatalogValidationReport,
    ReferenceModelCapabilityProfile, ReferenceModelEntry, ReferenceModelRecord,
    ReferenceProviderAuth, ReferenceProviderCapabilityProfile, ReferenceProviderDescriptor,
    ReferenceProviderModelCatalog, ResolvedEndpoint, ResolvedInvocation, RuntimeProviderApi,
    RuntimeProviderHints, RuntimeRoute, RuntimeRouteRequest, TransportKind, VerificationStatus,
    WireProtocol, builtin_provider_plugins, builtin_registry, capability_for_operation,
    core_provider_reference_catalog_expectations,
};
pub use config::{
    ConfigScope, Env, ModelConfig, ModelDeleteReport, ModelDeleteRequest, ModelListReport,
    ModelListRequest, ModelShowReport, ModelShowRequest, ModelSummary, ModelUpsertReport,
    ModelUpsertRequest, ProviderApi, ProviderAuth, ProviderAuthType, ProviderCapabilities,
    ProviderConfig, ProviderDeleteReport, ProviderDeleteRequest, ProviderListReport,
    ProviderListRequest, ProviderNamespace, ProviderRoutingConfig, ProviderShowReport,
    ProviderShowRequest, ProviderSummary, ProviderUpsertReport, ProviderUpsertRequest,
    ResolvedRoutingPlan, ResolvedRoutingTarget, RoutingConfigFormat, RoutingContext,
    RoutingOverride, RoutingPhase, RoutingPolicy, RoutingPolicySource, RoutingProviderProfile,
    RoutingStagePolicy, RoutingTarget, ThinkingIntensity,
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
    delete_model_config, delete_provider_config, filter_models_whitelist, list_model_configs,
    list_provider_configs, merge_provider_config, normalize_string_list, parse_dotenv,
    resolve_auth_token, resolve_auth_token_with_default_keys, select_model_config,
    show_model_config, show_provider_config, upsert_model_config, upsert_provider_config,
};
pub use core::{
    AbortableStream, CollectedStream, DittoError, LanguageModel, LanguageModelExt,
    LanguageModelLayer, LanguageModelLayerExt, LayeredLanguageModel, ProviderResolutionError,
    Result, StreamAbortHandle, StreamResult, abortable_stream, collect_stream,
};
#[doc(hidden)]
pub use profile::{
    AnthropicCatalogProvider, AnthropicModelCatalog, AnthropicModelCatalogEntry,
    AnthropicModelPricing, AnthropicModelStatus, GoogleCatalogProvider, GoogleModelCatalog,
    GoogleModelCatalogEntry, GoogleModelVersion, GoogleSupportedDataTypes, OpenAiCatalogProvider,
    OpenAiCompatibleClient, OpenAiModalitySupport, OpenAiModelCatalog, OpenAiModelCatalogEntry,
    OpenAiModelRevisions, OpenAiModelsProvider, Provider, anthropic_model_catalog,
    anthropic_model_catalog_entry, anthropic_model_catalog_entry_by_model, google_model_catalog,
    google_model_catalog_entry, google_model_catalog_entry_by_model, list_available_models,
    openai_model_catalog, openai_model_catalog_entry,
};
pub use providers::openai_compatible_family::{
    OpenAiProviderFamily, OpenAiProviderQuirks, PromptCacheUsageReporting,
    infer_openai_provider_family, infer_openai_provider_quirks,
};
pub use runtime::{
    BuiltinProviderCapabilitySummary, BuiltinProviderModelCandidate, BuiltinProviderPreset,
    ResolvedProviderCapabilityProfile, ResolvedProviderConfigSemantics, RuntimeCatalogResolver,
    builtin_models_for_provider, builtin_provider_candidates_for_model,
    builtin_provider_capability_summaries, builtin_provider_capability_summary,
    builtin_provider_preset, builtin_provider_presets, resolve_builtin_runtime_route,
    resolve_openai_compatible_provider_capability_profile, resolve_provider_config_semantics,
};

#[cfg(feature = "sdk")]
pub use sdk::cache::CacheLayer;
pub use types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchRequestCounts, BatchResponse, BatchStatus, ContentPart, FileSource,
    FinishReason, GenerateRequest, GenerateResponse, ImageEditRequest, ImageEditResponse,
    ImageEditUpload, ImageGenerationRequest, ImageGenerationResponse, ImageResponseFormat,
    ImageSource, JsonSchemaFormat, Message, ModerationInput, ModerationRequest, ModerationResponse,
    ModerationResult, ProviderOptions, ReasoningEffort, ReasoningSummary, RerankDocument,
    RerankRequest, RerankResponse, RerankResult, ResponseFormat, Role, SpeechRequest,
    SpeechResponse, SpeechResponseFormat, StreamChunk, Tool, ToolChoice,
    TranscriptionResponseFormat, Usage, VideoContentVariant, VideoDeleteResponse,
    VideoGenerationError, VideoGenerationRequest, VideoGenerationResponse, VideoGenerationStatus,
    VideoListOrder, VideoListRequest, VideoListResponse, VideoReferenceUpload, VideoRemixRequest,
    Warning,
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
#[cfg(all(feature = "google", feature = "images"))]
pub use providers::GoogleImages;
#[cfg(all(feature = "google", feature = "realtime"))]
pub use providers::GoogleRealtime;
#[cfg(all(feature = "google", feature = "videos"))]
pub use providers::GoogleVideos;
#[cfg(feature = "openai")]
pub use providers::OpenAI;
#[cfg(all(feature = "openai", feature = "batches"))]
pub use providers::OpenAIBatches;
#[cfg(all(feature = "openai", feature = "openai-compatible"))]
pub use providers::OpenAIChatCompletions;
#[cfg(feature = "openai-compatible")]
pub use providers::OpenAICompatible;
#[cfg(all(feature = "openai-compatible", feature = "batches"))]
pub use providers::OpenAICompatibleBatches;
#[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
pub use providers::OpenAICompatibleEmbeddings;
#[cfg(all(feature = "openai-compatible", feature = "moderations"))]
pub use providers::OpenAICompatibleModerations;
#[cfg(all(feature = "openai", feature = "embeddings"))]
pub use providers::OpenAIEmbeddings;
#[cfg(all(feature = "openai", feature = "moderations"))]
pub use providers::OpenAIModerations;
#[cfg(all(feature = "openai", feature = "realtime"))]
pub use providers::OpenAIRealtime;
#[cfg(all(feature = "openai", feature = "videos"))]
pub use providers::OpenAIVideos;
#[cfg(feature = "vertex")]
pub use providers::Vertex;
#[cfg(all(feature = "openai", feature = "audio"))]
pub use providers::{OpenAIAudioTranscription, OpenAISpeech};
#[cfg(all(feature = "openai-compatible", feature = "audio"))]
pub use providers::{OpenAICompatibleAudioTranscription, OpenAICompatibleSpeech};
#[cfg(all(feature = "openai-compatible", feature = "images"))]
pub use providers::{OpenAICompatibleImageEdits, OpenAICompatibleImages};
#[cfg(feature = "openai")]
pub use providers::{OpenAICompletionsLegacy, OpenAITextModel};
#[cfg(all(feature = "openai", feature = "images"))]
pub use providers::{OpenAIImageEdits, OpenAIImages};
