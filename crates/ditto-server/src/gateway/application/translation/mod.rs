// Gateway translation application implementation.
// inlined from ../../translation/backend.rs
mod endpoint_routing;
mod files_api;
mod openai_provider_options;
mod request_shaping;
mod response_mapping;
mod response_store;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use axum::http::StatusCode;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, OnceCell};

use crate::capabilities::BatchClient;
use crate::capabilities::audio::{AudioTranscriptionModel, SpeechModel};
use crate::capabilities::embedding::EmbeddingModel;
use crate::capabilities::file::{FileClient, FileContent, FileUploadRequest};
use crate::capabilities::video::VideoGenerationModel;
use crate::capabilities::{ImageEditModel, ImageGenerationModel, ModerationModel, RerankModel};
use crate::config::{Env, ProviderConfig};
use crate::contracts::{
    CapabilityKind, ContentPart, FinishReason, GenerateRequest, GenerateResponse, ImageSource,
    Message, OperationKind, Role, RuntimeRouteRequest, Usage,
};
use crate::error::DittoError;
use crate::gateway::adapters::cache::LocalLruCache;
use crate::llm_core::model::{LanguageModel, StreamResult};
use crate::object::{LanguageModelObjectExt, ObjectOptions, ObjectOutput};
use crate::provider_options::JsonSchemaFormat;
use crate::runtime::{
    build_audio_transcription_model, build_batch_client, build_embedding_model, build_file_client,
    build_image_edit_model, build_image_generation_model, build_moderation_model,
    build_rerank_model, build_speech_model, build_video_generation_model,
};
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, ImageEditRequest, ImageEditResponse, ImageGenerationRequest,
    ImageGenerationResponse, ModerationRequest, ModerationResponse, RerankRequest, RerankResponse,
    SpeechRequest, SpeechResponse, SpeechResponseFormat, TranscriptionResponseFormat,
    VideoContentVariant, VideoDeleteResponse, VideoGenerationRequest, VideoGenerationResponse,
    VideoListRequest, VideoListResponse, VideoRemixRequest,
};
pub use endpoint_routing::*;
pub use files_api::*;
use openai_provider_options::apply_openai_request_provider_options;
use response_mapping::{
    chat_chunk_bytes, chat_usage_chunk_bytes, completion_chunk_bytes,
    finish_reason_to_chat_finish_reason, finish_reason_to_responses_status, sse_event_bytes,
    usage_to_chat_usage, usage_to_responses_usage,
};
use response_store::TranslationResponseStore;
pub(crate) use response_store::{
    delete_stored_response_from_translation_backends,
    find_stored_response_from_translation_backends,
};

type ParseResult<T> = std::result::Result<T, String>;
type IoResult<T> = std::result::Result<T, std::io::Error>;

const DEFAULT_TRANSLATION_MODEL_CACHE_MAX_ENTRIES: usize = 64;
const MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES: usize = 256;

#[derive(Clone, Default)]
struct TranslationBackendBindings {
    embedding_model: Option<Arc<dyn EmbeddingModel>>,
    image_generation_model: Option<Arc<dyn ImageGenerationModel>>,
    image_edit_model: Option<Arc<dyn ImageEditModel>>,
    video_generation_model: Option<Arc<dyn VideoGenerationModel>>,
    moderation_model: Option<Arc<dyn ModerationModel>>,
    audio_transcription_model: Option<Arc<dyn AudioTranscriptionModel>>,
    speech_model: Option<Arc<dyn SpeechModel>>,
    rerank_model: Option<Arc<dyn RerankModel>>,
    batch_client: Option<Arc<dyn BatchClient>>,
    file_client: Option<Arc<dyn FileClient>>,
}

#[derive(Clone)]
struct TranslationBackendRuntime {
    model_cache_max_entries: usize,
    env: Env,
    provider_config: ProviderConfig,
    embedding_cache: Arc<Mutex<LocalLruCache<Arc<dyn EmbeddingModel>>>>,
    moderation_cache: Arc<OnceCell<Arc<dyn ModerationModel>>>,
    image_generation_cache: Arc<OnceCell<Arc<dyn ImageGenerationModel>>>,
    image_edit_cache: Arc<OnceCell<Arc<dyn ImageEditModel>>>,
    video_generation_cache: Arc<OnceCell<Arc<dyn VideoGenerationModel>>>,
    audio_transcription_cache: Arc<Mutex<LocalLruCache<Arc<dyn AudioTranscriptionModel>>>>,
    speech_cache: Arc<Mutex<LocalLruCache<Arc<dyn SpeechModel>>>>,
    rerank_cache: Arc<Mutex<LocalLruCache<Arc<dyn RerankModel>>>>,
    batch_cache: Arc<OnceCell<Arc<dyn BatchClient>>>,
    file_cache: Arc<OnceCell<Arc<dyn FileClient>>>,
    response_store: TranslationResponseStore,
}

impl Default for TranslationBackendRuntime {
    fn default() -> Self {
        Self {
            model_cache_max_entries: DEFAULT_TRANSLATION_MODEL_CACHE_MAX_ENTRIES,
            env: Env::default(),
            provider_config: ProviderConfig::default(),
            embedding_cache: Arc::new(Mutex::new(LocalLruCache::default())),
            moderation_cache: Arc::new(OnceCell::new()),
            image_generation_cache: Arc::new(OnceCell::new()),
            image_edit_cache: Arc::new(OnceCell::new()),
            video_generation_cache: Arc::new(OnceCell::new()),
            audio_transcription_cache: Arc::new(Mutex::new(LocalLruCache::default())),
            speech_cache: Arc::new(Mutex::new(LocalLruCache::default())),
            rerank_cache: Arc::new(Mutex::new(LocalLruCache::default())),
            batch_cache: Arc::new(OnceCell::new()),
            file_cache: Arc::new(OnceCell::new()),
            response_store: TranslationResponseStore::default(),
        }
    }
}

impl TranslationBackendRuntime {
    fn with_model_cache_max_entries(mut self, max_entries: usize) -> Self {
        self.model_cache_max_entries = max_entries;
        self
    }

    fn with_provider_config(mut self, provider_config: ProviderConfig) -> Self {
        self.provider_config = provider_config;
        self
    }

    fn with_env(mut self, env: Env) -> Self {
        self.env = env;
        self
    }

    fn configured_default_model(&self) -> Option<&str> {
        self.provider_config
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }

    fn supports_runtime_route(
        &self,
        provider: &str,
        model: Option<&str>,
        operation: OperationKind,
        capability: Option<CapabilityKind>,
    ) -> bool {
        let provider = provider.trim();
        if provider.is_empty() {
            return false;
        }

        let mut request = RuntimeRouteRequest::new(provider, model, operation)
            .with_runtime_hints(self.provider_config.runtime_hints());
        if let Some(capability) = capability {
            request = request.with_required_capability(capability);
        }

        crate::runtime::resolve_builtin_runtime_route(request).is_ok()
    }

    fn supports_runtime_capability(
        &self,
        provider: &str,
        model: Option<&str>,
        capability: CapabilityKind,
    ) -> bool {
        let provider = provider.trim();
        if provider.is_empty() {
            return false;
        }

        let requested_model = if capability == CapabilityKind::BATCH {
            None
        } else {
            model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| self.configured_default_model())
        };

        crate::runtime::builtin_runtime_supports_capability(
            provider,
            &self.provider_config,
            requested_model,
            capability,
        )
    }

    fn supports_file_builder(&self, provider: &str) -> bool {
        crate::runtime::builtin_runtime_supports_file_builder(provider, &self.provider_config)
    }

    async fn resolve_embedding_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn EmbeddingModel>>,
        model: &str,
    ) -> crate::error::Result<Arc<dyn EmbeddingModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::invalid_response_text(
                "embedding model is missing",
            ));
        }

        let cacheable = model.len() <= MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES
            && self.model_cache_max_entries > 0;
        if cacheable {
            let cached = { self.embedding_cache.lock().await.get(model) };
            if let Some(model_impl) = cached {
                return Ok(model_impl);
            }
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let model_impl = build_embedding_model(provider, &cfg, &self.env)
            .await?
            .ok_or_else(|| {
                DittoError::invalid_response_text(format!(
                    "provider backend does not support embeddings: {provider}"
                ))
            })?;

        if cacheable {
            let mut cache = self.embedding_cache.lock().await;
            cache.insert(
                model.to_string(),
                model_impl.clone(),
                self.model_cache_max_entries,
            );
        }

        Ok(model_impl)
    }

    async fn resolve_moderation_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn ModerationModel>>,
    ) -> crate::error::Result<Arc<dyn ModerationModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let provider = provider.trim().to_string();
        let model_impl = self
            .moderation_cache
            .get_or_try_init(|| async {
                build_moderation_model(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support moderations: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(model_impl.clone())
    }

    async fn resolve_image_generation_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn ImageGenerationModel>>,
    ) -> crate::error::Result<Arc<dyn ImageGenerationModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let provider = provider.trim().to_string();
        let model_impl = self
            .image_generation_cache
            .get_or_try_init(|| async {
                build_image_generation_model(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support images: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(model_impl.clone())
    }

    async fn resolve_image_edit_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn ImageEditModel>>,
    ) -> crate::error::Result<Arc<dyn ImageEditModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let provider = provider.trim().to_string();
        let model_impl = self
            .image_edit_cache
            .get_or_try_init(|| async {
                build_image_edit_model(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support image edits: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(model_impl.clone())
    }

    async fn resolve_video_generation_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn VideoGenerationModel>>,
    ) -> crate::error::Result<Arc<dyn VideoGenerationModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let provider = provider.trim().to_string();
        let model_impl = self
            .video_generation_cache
            .get_or_try_init(|| async {
                build_video_generation_model(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support videos: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(model_impl.clone())
    }

    async fn resolve_audio_transcription_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn AudioTranscriptionModel>>,
        model: &str,
    ) -> crate::error::Result<Arc<dyn AudioTranscriptionModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::invalid_response_text(
                "audio transcription model is missing",
            ));
        }

        let cacheable = model.len() <= MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES
            && self.model_cache_max_entries > 0;
        if cacheable {
            let cached = { self.audio_transcription_cache.lock().await.get(model) };
            if let Some(model_impl) = cached {
                return Ok(model_impl);
            }
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let model_impl = build_audio_transcription_model(provider, &cfg, &self.env)
            .await?
            .ok_or_else(|| {
                DittoError::invalid_response_text(format!(
                    "provider backend does not support audio transcriptions: {provider}"
                ))
            })?;

        if cacheable {
            let mut cache = self.audio_transcription_cache.lock().await;
            cache.insert(
                model.to_string(),
                model_impl.clone(),
                self.model_cache_max_entries,
            );
        }

        Ok(model_impl)
    }

    async fn resolve_speech_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn SpeechModel>>,
        model: &str,
    ) -> crate::error::Result<Arc<dyn SpeechModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::invalid_response_text("speech model is missing"));
        }

        let cacheable = model.len() <= MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES
            && self.model_cache_max_entries > 0;
        if cacheable {
            let cached = { self.speech_cache.lock().await.get(model) };
            if let Some(model_impl) = cached {
                return Ok(model_impl);
            }
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let model_impl = build_speech_model(provider, &cfg, &self.env)
            .await?
            .ok_or_else(|| {
                DittoError::invalid_response_text(format!(
                    "provider backend does not support audio speech: {provider}"
                ))
            })?;

        if cacheable {
            let mut cache = self.speech_cache.lock().await;
            cache.insert(
                model.to_string(),
                model_impl.clone(),
                self.model_cache_max_entries,
            );
        }

        Ok(model_impl)
    }

    async fn resolve_rerank_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn RerankModel>>,
        model: &str,
    ) -> crate::error::Result<Arc<dyn RerankModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::invalid_response_text("rerank model is missing"));
        }

        let cacheable = model.len() <= MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES
            && self.model_cache_max_entries > 0;
        if cacheable {
            let cached = { self.rerank_cache.lock().await.get(model) };
            if let Some(model_impl) = cached {
                return Ok(model_impl);
            }
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let model_impl = build_rerank_model(provider, &cfg, &self.env)
            .await?
            .ok_or_else(|| {
                DittoError::invalid_response_text(format!(
                    "provider backend does not support rerank: {provider}"
                ))
            })?;

        if cacheable {
            let mut cache = self.rerank_cache.lock().await;
            cache.insert(
                model.to_string(),
                model_impl.clone(),
                self.model_cache_max_entries,
            );
        }

        Ok(model_impl)
    }

    async fn resolve_batch_client(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn BatchClient>>,
    ) -> crate::error::Result<Arc<dyn BatchClient>> {
        if let Some(client) = direct.cloned() {
            return Ok(client);
        }

        let provider = provider.trim().to_string();
        let client = self
            .batch_cache
            .get_or_try_init(|| async {
                build_batch_client(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support batches: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(client.clone())
    }

    async fn resolve_file_client(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn FileClient>>,
    ) -> crate::error::Result<Arc<dyn FileClient>> {
        if let Some(client) = direct.cloned() {
            return Ok(client);
        }

        let provider = provider.trim().to_string();
        let client = self
            .file_cache
            .get_or_try_init(|| async {
                build_file_client(provider.as_str(), &self.provider_config, &self.env)
                    .await?
                    .ok_or_else(|| {
                        DittoError::invalid_response_text(format!(
                            "provider backend does not support files: {provider}"
                        ))
                    })
            })
            .await?;

        Ok(client.clone())
    }
}

#[derive(Clone)]
pub struct TranslationBackend {
    pub model: Arc<dyn LanguageModel>,
    pub provider: String,
    pub model_map: BTreeMap<String, String>,
    bindings: TranslationBackendBindings,
    runtime: TranslationBackendRuntime,
}

impl TranslationBackend {
    pub fn new(provider: impl Into<String>, model: Arc<dyn LanguageModel>) -> Self {
        Self {
            model,
            provider: provider.into(),
            model_map: BTreeMap::new(),
            bindings: TranslationBackendBindings::default(),
            runtime: TranslationBackendRuntime::default(),
        }
    }

    pub fn with_model_cache_max_entries(mut self, max_entries: usize) -> Self {
        self.runtime = self.runtime.with_model_cache_max_entries(max_entries);
        self
    }

    pub fn with_provider_config(mut self, provider_config: ProviderConfig) -> Self {
        self.runtime = self.runtime.with_provider_config(provider_config);
        self
    }

    pub fn with_env(mut self, env: Env) -> Self {
        self.runtime = self.runtime.with_env(env);
        self
    }

    pub fn with_model_map(mut self, model_map: BTreeMap<String, String>) -> Self {
        self.model_map = model_map;
        self
    }

    pub fn with_embedding_model(mut self, embedding_model: Arc<dyn EmbeddingModel>) -> Self {
        self.bindings.embedding_model = Some(embedding_model);
        self
    }

    pub fn with_image_generation_model(
        mut self,
        image_generation_model: Arc<dyn ImageGenerationModel>,
    ) -> Self {
        self.bindings.image_generation_model = Some(image_generation_model);
        self
    }

    pub fn with_image_edit_model(mut self, image_edit_model: Arc<dyn ImageEditModel>) -> Self {
        self.bindings.image_edit_model = Some(image_edit_model);
        self
    }

    pub fn with_video_generation_model(
        mut self,
        video_generation_model: Arc<dyn VideoGenerationModel>,
    ) -> Self {
        self.bindings.video_generation_model = Some(video_generation_model);
        self
    }

    pub fn with_moderation_model(mut self, moderation_model: Arc<dyn ModerationModel>) -> Self {
        self.bindings.moderation_model = Some(moderation_model);
        self
    }

    pub fn with_audio_transcription_model(
        mut self,
        audio_transcription_model: Arc<dyn AudioTranscriptionModel>,
    ) -> Self {
        self.bindings.audio_transcription_model = Some(audio_transcription_model);
        self
    }

    pub fn with_speech_model(mut self, speech_model: Arc<dyn SpeechModel>) -> Self {
        self.bindings.speech_model = Some(speech_model);
        self
    }

    pub fn with_rerank_model(mut self, rerank_model: Arc<dyn RerankModel>) -> Self {
        self.bindings.rerank_model = Some(rerank_model);
        self
    }

    pub fn with_batch_client(mut self, batch_client: Arc<dyn BatchClient>) -> Self {
        self.bindings.batch_client = Some(batch_client);
        self
    }

    pub fn with_file_client(mut self, file_client: Arc<dyn FileClient>) -> Self {
        self.bindings.file_client = Some(file_client);
        self
    }

    pub fn provider_name(&self) -> &str {
        self.provider.trim()
    }

    pub fn default_model_id(&self) -> &str {
        self.model.model_id().trim()
    }

    fn bound_supports_capability(&self, capability: CapabilityKind) -> bool {
        match capability {
            CapabilityKind::LLM => true,
            CapabilityKind::EMBEDDING => self.bindings.embedding_model.is_some(),
            CapabilityKind::IMAGE_GENERATION => self.bindings.image_generation_model.is_some(),
            CapabilityKind::IMAGE_EDIT => self.bindings.image_edit_model.is_some(),
            CapabilityKind::VIDEO_GENERATION => self.bindings.video_generation_model.is_some(),
            CapabilityKind::MODERATION => self.bindings.moderation_model.is_some(),
            CapabilityKind::AUDIO_TRANSCRIPTION | CapabilityKind::AUDIO_TRANSLATION => {
                self.bindings.audio_transcription_model.is_some()
            }
            CapabilityKind::AUDIO_SPEECH => self.bindings.speech_model.is_some(),
            CapabilityKind::RERANK => self.bindings.rerank_model.is_some(),
            CapabilityKind::BATCH => self.bindings.batch_client.is_some(),
            _ => false,
        }
    }

    fn bound_supports_runtime_operation(&self, operation: OperationKind) -> bool {
        match operation {
            OperationKind::CHAT_COMPLETION
            | OperationKind::RESPONSE
            | OperationKind::TEXT_COMPLETION
            | OperationKind::THREAD_RUN
            | OperationKind::GROUP_CHAT_COMPLETION
            | OperationKind::CHAT_TRANSLATION => true,
            OperationKind::EMBEDDING | OperationKind::MULTIMODAL_EMBEDDING => {
                self.bindings.embedding_model.is_some()
            }
            OperationKind::IMAGE_GENERATION => self.bindings.image_generation_model.is_some(),
            OperationKind::IMAGE_EDIT => self.bindings.image_edit_model.is_some(),
            OperationKind::VIDEO_GENERATION => self.bindings.video_generation_model.is_some(),
            OperationKind::MODERATION => self.bindings.moderation_model.is_some(),
            OperationKind::AUDIO_TRANSCRIPTION | OperationKind::AUDIO_TRANSLATION => {
                self.bindings.audio_transcription_model.is_some()
            }
            OperationKind::AUDIO_SPEECH => self.bindings.speech_model.is_some(),
            OperationKind::RERANK => self.bindings.rerank_model.is_some(),
            OperationKind::BATCH => self.bindings.batch_client.is_some(),
            _ => false,
        }
    }

    fn supports_files_api(&self) -> bool {
        self.bindings.file_client.is_some()
            || self.runtime.supports_file_builder(self.provider_name())
    }

    pub fn supports_endpoint(
        &self,
        descriptor: &TranslationEndpointDescriptor,
        model: Option<&str>,
    ) -> bool {
        match descriptor.requirement {
            TranslationEndpointRequirement::None => true,
            TranslationEndpointRequirement::FilesApi => self.supports_files_api(),
            TranslationEndpointRequirement::RuntimeCapability(capabilities) => self
                .supports_runtime_capabilities(descriptor.runtime_operation, capabilities, model),
        }
    }

    fn supports_runtime_capabilities(
        &self,
        operation: Option<OperationKind>,
        capabilities: &'static [CapabilityKind],
        model: Option<&str>,
    ) -> bool {
        let model = model.map(str::trim).filter(|value| !value.is_empty());

        if let Some(operation) = operation
            && self.bound_supports_runtime_operation(operation)
            && (capabilities.is_empty()
                || capabilities
                    .iter()
                    .copied()
                    .any(|capability| self.bound_supports_capability(capability)))
        {
            return true;
        }

        if let Some(operation) = operation {
            if capabilities.is_empty() {
                return self.runtime.supports_runtime_route(
                    self.provider_name(),
                    model,
                    operation,
                    None,
                );
            }
            return capabilities.iter().copied().any(|capability| {
                self.runtime.supports_runtime_route(
                    self.provider_name(),
                    model,
                    operation,
                    Some(capability),
                )
            });
        }

        capabilities.iter().copied().any(|capability| {
            self.runtime
                .supports_runtime_capability(self.provider_name(), model, capability)
        })
    }

    pub fn map_model(&self, requested: &str) -> String {
        if let Some(mapped) = self.model_map.get(requested) {
            return mapped.clone();
        }

        let requested = requested.trim();
        if requested.is_empty() {
            return String::new();
        }

        let prefix = format!("{}/", self.provider_name());
        if prefix != "/" && requested.starts_with(&prefix) {
            return requested.trim_start_matches(&prefix).to_string();
        }

        requested.to_string()
    }

    pub async fn upload_file(&self, request: FileUploadRequest) -> crate::error::Result<String> {
        let client = self.resolve_file_client().await?;
        client.upload_file_with_purpose(request).await
    }

    pub async fn embed(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> crate::error::Result<Vec<Vec<f32>>> {
        let model_impl = self
            .runtime
            .resolve_embedding_model(
                self.provider_name(),
                self.bindings.embedding_model.as_ref(),
                model,
            )
            .await?;

        model_impl.embed(texts).await
    }

    pub async fn moderate(
        &self,
        request: ModerationRequest,
    ) -> crate::error::Result<ModerationResponse> {
        let model_impl = self
            .runtime
            .resolve_moderation_model(
                self.provider_name(),
                self.bindings.moderation_model.as_ref(),
            )
            .await?;

        model_impl.moderate(request).await
    }

    pub async fn generate_image(
        &self,
        request: ImageGenerationRequest,
    ) -> crate::error::Result<ImageGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_image_generation_model(
                self.provider_name(),
                self.bindings.image_generation_model.as_ref(),
            )
            .await?;

        model_impl.generate(request).await
    }

    pub async fn edit_image(
        &self,
        request: ImageEditRequest,
    ) -> crate::error::Result<ImageEditResponse> {
        let model_impl = self
            .runtime
            .resolve_image_edit_model(
                self.provider_name(),
                self.bindings.image_edit_model.as_ref(),
            )
            .await?;

        model_impl.edit(request).await
    }

    pub async fn create_video(
        &self,
        request: VideoGenerationRequest,
    ) -> crate::error::Result<VideoGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.create(request).await
    }

    pub async fn retrieve_video(
        &self,
        video_id: &str,
    ) -> crate::error::Result<VideoGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.retrieve(video_id).await
    }

    pub async fn list_videos(
        &self,
        request: VideoListRequest,
    ) -> crate::error::Result<VideoListResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.list(request).await
    }

    pub async fn delete_video(&self, video_id: &str) -> crate::error::Result<VideoDeleteResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.delete(video_id).await
    }

    pub async fn download_video_content(
        &self,
        video_id: &str,
        variant: Option<VideoContentVariant>,
    ) -> crate::error::Result<FileContent> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.download_content(video_id, variant).await
    }

    pub async fn remix_video(
        &self,
        video_id: &str,
        request: VideoRemixRequest,
    ) -> crate::error::Result<VideoGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.remix(video_id, request).await
    }

    pub async fn transcribe_audio(
        &self,
        model: &str,
        mut request: AudioTranscriptionRequest,
    ) -> crate::error::Result<AudioTranscriptionResponse> {
        let model_impl = self
            .runtime
            .resolve_audio_transcription_model(
                self.provider_name(),
                self.bindings.audio_transcription_model.as_ref(),
                model,
            )
            .await?;

        if request
            .model
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            request.model = Some(model.trim().to_string());
        }
        model_impl.transcribe(request).await
    }

    pub async fn speak_audio(
        &self,
        model: &str,
        mut request: SpeechRequest,
    ) -> crate::error::Result<SpeechResponse> {
        let model_impl = self
            .runtime
            .resolve_speech_model(
                self.provider_name(),
                self.bindings.speech_model.as_ref(),
                model,
            )
            .await?;

        if request
            .model
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            request.model = Some(model.trim().to_string());
        }
        model_impl.speak(request).await
    }

    pub async fn rerank(
        &self,
        model: &str,
        mut request: RerankRequest,
    ) -> crate::error::Result<RerankResponse> {
        let model_impl = self
            .runtime
            .resolve_rerank_model(
                self.provider_name(),
                self.bindings.rerank_model.as_ref(),
                model,
            )
            .await?;

        if request
            .model
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            request.model = Some(model.trim().to_string());
        }
        model_impl.rerank(request).await
    }

    pub async fn create_batch(
        &self,
        request: BatchCreateRequest,
    ) -> crate::error::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.create(request).await
    }

    pub async fn retrieve_batch(&self, batch_id: &str) -> crate::error::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.retrieve(batch_id).await
    }

    pub async fn cancel_batch(&self, batch_id: &str) -> crate::error::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.cancel(batch_id).await
    }

    pub async fn list_batches(
        &self,
        limit: Option<u32>,
        after: Option<String>,
    ) -> crate::error::Result<BatchListResponse> {
        let client = self.resolve_batch_client().await?;
        client.list(limit, after).await
    }

    async fn resolve_batch_client(&self) -> crate::error::Result<Arc<dyn BatchClient>> {
        self.runtime
            .resolve_batch_client(self.provider_name(), self.bindings.batch_client.as_ref())
            .await
    }

    pub async fn compact_responses_history(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
    ) -> crate::error::Result<(Vec<Value>, Usage)> {
        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::invalid_response_text(
                "compaction model is missing",
            ));
        }

        let instructions = instructions.trim();
        let mut system = String::new();
        if !instructions.is_empty() {
            system.push_str(instructions);
            system.push_str("\n\n");
        }
        system.push_str(
            concat!(
                "You are a compaction helper for the OpenAI Responses API.\n\n",
                "Goal: return a compacted version of the provided input history as OpenAI Responses input items.\n",
                "- Preserve the user's goals, constraints, and important context.\n",
                "- Preserve tool outputs only if still relevant; drop redundant/low-value details.\n",
                "- Do not invent facts.\n",
                "- Output MUST be a JSON array of objects (Responses input items).\n",
            ),
        );

        let input_json = serde_json::to_string(input).unwrap_or_else(|_| "[]".to_string());
        let request = GenerateRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: vec![ContentPart::Text { text: system }],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentPart::Text {
                        text: format!(
                            "Compact the following OpenAI Responses input items JSON:\n{input_json}"
                        ),
                    }],
                },
            ],
            model: Some(model.to_string()),
            temperature: Some(0.0),
            max_tokens: None,
            top_p: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            logprobs: None,
            top_logprobs: None,
            user: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
            provider_options: None,
        };

        let schema = JsonSchemaFormat {
            name: "responses_compacted_input_items".to_string(),
            schema: serde_json::json!({"type":"object"}),
            strict: None,
        };

        let out = self
            .model
            .generate_object_json_with(
                request,
                schema,
                ObjectOptions {
                    output: ObjectOutput::Array,
                    ..ObjectOptions::default()
                },
            )
            .await?;

        let Value::Array(items) = out.object else {
            return Err(DittoError::invalid_response_text(
                "compaction response is not a JSON array",
            ));
        };

        let mut usage = out.response.usage;
        usage.merge_total();
        Ok((items, usage))
    }
}
// end inline: ../../translation/backend.rs
pub fn collect_models_from_translation_backends(
    backends: &HashMap<String, TranslationBackend>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();

    let mut backend_names = backends.keys().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let backend = match backends.get(backend_name) {
            Some(backend) => backend,
            None => continue,
        };

        let provider = backend.provider_name();
        let owned_by = if provider.is_empty() {
            backend_name.as_str()
        } else {
            provider
        };

        for key in backend.model_map.keys() {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            out.entry(key.to_string())
                .or_insert_with(|| owned_by.to_string());
        }

        let default_model = backend.default_model_id();
        if !default_model.is_empty() {
            out.entry(format!("{owned_by}/{default_model}"))
                .or_insert_with(|| owned_by.to_string());
        }

        for value in backend.model_map.values() {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            out.entry(format!("{owned_by}/{value}"))
                .or_insert_with(|| owned_by.to_string());
        }
    }

    out
}

pub fn model_to_openai(id: &str, owned_by: &str, created: u64) -> Value {
    let id = id.trim();
    let owned_by = owned_by.trim();
    serde_json::json!({
        "id": id,
        "object": "model",
        "created": created,
        "owned_by": owned_by,
    })
}

pub fn models_list_to_openai(models: &BTreeMap<String, String>, created: u64) -> Value {
    let data = models
        .iter()
        .map(|(id, owned_by)| model_to_openai(id, owned_by, created))
        .collect::<Vec<_>>();
    serde_json::json!({
        "object": "list",
        "data": data,
    })
}

pub fn batches_create_request_to_request(request: &Value) -> ParseResult<BatchCreateRequest> {
    serde_json::from_value::<BatchCreateRequest>(request.clone())
        .map_err(|err| format!("batches request is invalid: {err}"))
}

pub fn batch_to_openai(batch: &Batch) -> Value {
    let mut value = serde_json::to_value(batch).unwrap_or(Value::Null);
    if let Value::Object(obj) = &mut value {
        obj.insert("object".to_string(), Value::String("batch".to_string()));
    }
    value
}

pub fn batch_list_response_to_openai(response: &BatchListResponse) -> Value {
    let mut obj = Map::<String, Value>::new();
    obj.insert("object".to_string(), Value::String("list".to_string()));

    let data: Vec<Value> = response.batches.iter().map(batch_to_openai).collect();
    obj.insert("data".to_string(), Value::Array(data));

    if let Some(has_more) = response.has_more {
        obj.insert("has_more".to_string(), Value::Bool(has_more));
    }

    let first_id = response
        .batches
        .first()
        .map(|batch| batch.id.trim().to_string())
        .filter(|id| !id.is_empty());
    if let Some(first_id) = first_id {
        obj.insert("first_id".to_string(), Value::String(first_id));
    }

    let last_id = response
        .batches
        .last()
        .map(|batch| batch.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .or_else(|| response.after.clone());
    if let Some(last_id) = last_id {
        obj.insert("last_id".to_string(), Value::String(last_id));
    }

    Value::Object(obj)
}

pub fn rerank_request_to_request(request: &Value) -> ParseResult<RerankRequest> {
    serde_json::from_value::<RerankRequest>(request.clone())
        .map_err(|err| format!("rerank request is invalid: {err}"))
}

pub fn rerank_response_to_openai(response: &RerankResponse) -> Value {
    let mut obj = Map::<String, Value>::new();

    if let Some(metadata) = response.provider_metadata.as_ref() {
        if let Some(id) = metadata.get("id") {
            obj.insert("id".to_string(), id.clone());
        }
        if let Some(meta) = metadata.get("meta") {
            obj.insert("meta".to_string(), meta.clone());
        }
    }

    let results: Vec<Value> = response
        .ranking
        .iter()
        .map(|result| {
            serde_json::json!({
                "index": result.index,
                "relevance_score": result.relevance_score,
            })
        })
        .collect();
    obj.insert("results".to_string(), Value::Array(results));

    Value::Object(obj)
}

pub fn multipart_extract_text_field(
    content_type: &str,
    body: &Bytes,
    field_name: &str,
) -> ParseResult<Option<String>> {
    request_shaping::multipart_extract_text_field(content_type, body, field_name)
}

pub fn audio_transcriptions_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<AudioTranscriptionRequest> {
    request_shaping::audio_transcriptions_request_to_request(content_type, body)
}

pub fn audio_speech_request_to_request(request: &Value) -> ParseResult<SpeechRequest> {
    request_shaping::audio_speech_request_to_request(request)
}

pub fn speech_response_format_to_content_type(
    format: Option<SpeechResponseFormat>,
) -> &'static str {
    request_shaping::speech_response_format_to_content_type(format)
}

pub fn transcription_format_to_content_type(
    format: Option<TranscriptionResponseFormat>,
) -> (&'static str, bool) {
    request_shaping::transcription_format_to_content_type(format)
}

pub fn chat_completions_request_to_generate_request(
    request: &Value,
) -> ParseResult<GenerateRequest> {
    request_shaping::chat_completions_request_to_generate_request(request)
}

pub fn completions_request_to_generate_request(request: &Value) -> ParseResult<GenerateRequest> {
    request_shaping::completions_request_to_generate_request(request)
}

pub fn embeddings_request_to_texts(request: &Value) -> ParseResult<Vec<String>> {
    request_shaping::embeddings_request_to_texts(request)
}

pub fn moderations_request_to_request(request: &Value) -> ParseResult<ModerationRequest> {
    request_shaping::moderations_request_to_request(request)
}

pub fn moderation_response_to_openai(response: &ModerationResponse, fallback_id: &str) -> Value {
    let results = response
        .results
        .iter()
        .map(|result| {
            serde_json::json!({
                "flagged": result.flagged,
                "categories": result.categories,
                "category_scores": result.category_scores,
            })
        })
        .collect::<Vec<_>>();

    let mut out = Map::<String, Value>::new();
    out.insert(
        "id".to_string(),
        Value::String(
            response
                .id
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(fallback_id)
                .to_string(),
        ),
    );
    if let Some(model) = response.model.as_deref().filter(|v| !v.trim().is_empty()) {
        out.insert("model".to_string(), Value::String(model.to_string()));
    }
    out.insert("results".to_string(), Value::Array(results));
    Value::Object(out)
}

pub fn images_generation_request_to_request(
    request: &Value,
) -> ParseResult<ImageGenerationRequest> {
    request_shaping::images_generation_request_to_request(request)
}

pub fn images_edits_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<ImageEditRequest> {
    request_shaping::images_edits_request_to_request(content_type, body)
}

pub fn responses_input_items_from_value(input: &Value) -> ParseResult<Vec<Value>> {
    request_shaping::responses_input_items_from_value(input)
}

pub fn videos_create_request_to_request(request: &Value) -> ParseResult<VideoGenerationRequest> {
    request_shaping::videos_create_request_to_request(request)
}

pub fn videos_create_multipart_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<VideoGenerationRequest> {
    request_shaping::videos_create_multipart_request_to_request(content_type, body)
}

pub fn videos_remix_request_to_request(request: &Value) -> ParseResult<VideoRemixRequest> {
    request_shaping::videos_remix_request_to_request(request)
}

pub fn videos_content_variant_from_path(
    path_and_query: &str,
) -> ParseResult<Option<VideoContentVariant>> {
    request_shaping::videos_content_variant_from_path(path_and_query)
}

pub fn videos_list_request_from_path(path_and_query: &str) -> ParseResult<VideoListRequest> {
    request_shaping::videos_list_request_from_path(path_and_query)
}

fn video_generation_response_to_openai_value(response: &VideoGenerationResponse) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert(
        "id".to_string(),
        Value::String(response.id.trim().to_string()),
    );
    out.insert(
        "object".to_string(),
        Value::String(
            response
                .object
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("video")
                .to_string(),
        ),
    );
    out.insert(
        "status".to_string(),
        serde_json::to_value(response.status)
            .unwrap_or_else(|_| Value::String("unknown".to_string())),
    );
    if let Some(model) = response
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert("model".to_string(), Value::String(model.to_string()));
    }
    if let Some(created_at) = response.created_at {
        out.insert(
            "created_at".to_string(),
            Value::Number((created_at as i64).into()),
        );
    }
    if let Some(completed_at) = response.completed_at {
        out.insert(
            "completed_at".to_string(),
            Value::Number((completed_at as i64).into()),
        );
    }
    if let Some(expires_at) = response.expires_at {
        out.insert(
            "expires_at".to_string(),
            Value::Number((expires_at as i64).into()),
        );
    }
    if let Some(progress) = response.progress {
        out.insert(
            "progress".to_string(),
            Value::Number((progress as i64).into()),
        );
    }
    if let Some(prompt) = response
        .prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert("prompt".to_string(), Value::String(prompt.to_string()));
    }
    if let Some(video_id) = response
        .remixed_from_video_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert(
            "remixed_from_video_id".to_string(),
            Value::String(video_id.to_string()),
        );
    }
    if let Some(seconds) = response
        .seconds
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert("seconds".to_string(), Value::String(seconds.to_string()));
    }
    if let Some(size) = response
        .size
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert("size".to_string(), Value::String(size.to_string()));
    }
    if let Some(error) = response.error.as_ref() {
        out.insert(
            "error".to_string(),
            serde_json::to_value(error).unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

pub fn video_generation_response_to_openai(response: &VideoGenerationResponse) -> Value {
    video_generation_response_to_openai_value(response)
}

pub fn video_list_response_to_openai(response: &VideoListResponse) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert("object".to_string(), Value::String("list".to_string()));
    out.insert(
        "data".to_string(),
        Value::Array(
            response
                .videos
                .iter()
                .map(video_generation_response_to_openai_value)
                .collect(),
        ),
    );
    if let Some(has_more) = response.has_more {
        out.insert("has_more".to_string(), Value::Bool(has_more));
    }
    if let Some(last_id) = response
        .after
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.insert("last_id".to_string(), Value::String(last_id.to_string()));
    }
    Value::Object(out)
}

pub fn video_delete_response_to_openai(response: &VideoDeleteResponse) -> Value {
    serde_json::json!({
        "id": response.id,
        "deleted": response.deleted,
        "object": response.object.as_deref().unwrap_or("video.deleted"),
    })
}

pub fn responses_input_tokens_to_openai(input_tokens: u32) -> Value {
    serde_json::json!({
        "object": "response.input_tokens",
        "input_tokens": input_tokens,
    })
}

pub fn responses_input_items_to_openai(input_items: &[Value]) -> Value {
    serde_json::json!({
        "object": "list",
        "data": input_items,
    })
}

pub fn response_delete_to_openai(response_id: &str) -> Value {
    serde_json::json!({
        "id": response_id,
        "object": "response",
        "deleted": true,
    })
}

pub fn image_generation_response_to_openai(
    response: &ImageGenerationResponse,
    created: u64,
) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );

    let data = response
        .images
        .iter()
        .map(|image| match image {
            ImageSource::Url { url } => serde_json::json!({ "url": url }),
            ImageSource::Base64 { data, .. } => serde_json::json!({ "b64_json": data }),
        })
        .collect::<Vec<_>>();
    out.insert("data".to_string(), Value::Array(data));
    Value::Object(out)
}

pub fn responses_request_to_generate_request(request: &Value) -> ParseResult<GenerateRequest> {
    let chat = crate::gateway::responses_shim::responses_request_to_chat_completions(request)
        .ok_or_else(|| "responses request cannot be mapped to chat/completions".to_string())?;
    let mut out = chat_completions_request_to_generate_request(&chat)?;

    let obj = request
        .as_object()
        .ok_or_else(|| "responses request must be a JSON object".to_string())?;

    apply_openai_request_provider_options(&mut out, obj)?;

    Ok(out)
}

pub fn embeddings_to_openai_response(embeddings: Vec<Vec<f32>>, model: &str) -> Value {
    fn safe_number(value: f32) -> Value {
        let num = serde_json::Number::from_f64(f64::from(value))
            .or_else(|| serde_json::Number::from_f64(0.0))
            .unwrap_or_else(|| serde_json::Number::from(0));
        Value::Number(num)
    }

    let mut data = Vec::<Value>::with_capacity(embeddings.len());
    for (index, embedding) in embeddings.into_iter().enumerate() {
        let vec = embedding.into_iter().map(safe_number).collect::<Vec<_>>();
        data.push(serde_json::json!({
            "object": "embedding",
            "index": index,
            "embedding": vec,
        }));
    }

    serde_json::json!({
        "object": "list",
        "data": data,
        "model": model,
    })
}
// end inline: ../../translation/openai_endpoints.rs
// inlined from ../../translation/openai_protocol.rs

pub fn generate_response_to_chat_completions(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut tool_calls = Vec::<Value>::new();
    for (idx, part) in response.content.iter().enumerate() {
        match part {
            ContentPart::Text { text } => content.push_str(text),
            ContentPart::Reasoning { text } => reasoning_content.push_str(text),
            ContentPart::ToolCall {
                id: call_id,
                name,
                arguments,
            } => {
                let call_id = call_id.trim();
                let call_id = if call_id.is_empty() {
                    format!("call_{idx}")
                } else {
                    call_id.to_string()
                };
                let arguments = arguments.to_string();
                tool_calls.push(serde_json::json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
            _ => {}
        }
    }

    let mut message = Map::<String, Value>::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    if !content.is_empty() {
        message.insert("content".to_string(), Value::String(content));
    } else {
        message.insert("content".to_string(), Value::Null);
    }
    if !reasoning_content.is_empty() {
        message.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content),
        );
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    let finish_reason = finish_reason_to_chat_finish_reason(response.finish_reason);

    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("message".to_string(), Value::Object(message));
    if let Some(finish_reason) = finish_reason {
        choice.insert(
            "finish_reason".to_string(),
            Value::String(finish_reason.to_string()),
        );
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert(
        "choices".to_string(),
        Value::Array(vec![Value::Object(choice)]),
    );

    if let Some(usage) = usage_to_chat_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }

    Value::Object(out)
}

pub fn generate_response_to_completions(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut text = String::new();
    for part in &response.content {
        if let ContentPart::Text { text: delta } = part {
            text.push_str(delta);
        }
    }

    let finish_reason = finish_reason_to_chat_finish_reason(response.finish_reason);

    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("text".to_string(), Value::String(text));
    choice.insert("logprobs".to_string(), Value::Null);
    if let Some(finish_reason) = finish_reason {
        choice.insert(
            "finish_reason".to_string(),
            Value::String(finish_reason.to_string()),
        );
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("text_completion".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert(
        "choices".to_string(),
        Value::Array(vec![Value::Object(choice)]),
    );
    if let Some(usage) = usage_to_chat_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }

    Value::Object(out)
}

pub fn generate_response_to_responses(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut output_text = String::new();
    let mut output_items = Vec::<Value>::new();
    let mut tool_calls = Vec::<Value>::new();

    for (idx, part) in response.content.iter().enumerate() {
        match part {
            ContentPart::Text { text } => output_text.push_str(text),
            ContentPart::ToolCall {
                id: call_id,
                name,
                arguments,
            } => {
                let call_id = call_id.trim();
                let call_id = if call_id.is_empty() {
                    format!("call_{idx}")
                } else {
                    call_id.to_string()
                };
                tool_calls.push(serde_json::json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments.to_string(),
                }));
            }
            _ => {}
        }
    }

    if !output_text.is_empty() {
        output_items.push(serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type":"output_text", "text": output_text}],
        }));
    }
    output_items.extend(tool_calls);

    let (status, incomplete_details) = finish_reason_to_responses_status(response.finish_reason);

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert("object".to_string(), Value::String("response".to_string()));
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("status".to_string(), Value::String(status.to_string()));
    if let Some(details) = incomplete_details {
        out.insert("incomplete_details".to_string(), details);
    }
    out.insert("output".to_string(), Value::Array(output_items));
    out.insert("output_text".to_string(), Value::String(output_text));
    if let Some(usage) = usage_to_responses_usage(&response.usage) {
        out.insert("usage".to_string(), usage);
    }
    Value::Object(out)
}

pub fn stream_to_chat_completions_sse(
    stream: StreamResult,
    fallback_id: String,
    model: String,
    created: u64,
    include_usage: bool,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct State {
        response_id: String,
        tool_call_index: HashMap<String, usize>,
        finish_reason: Option<FinishReason>,
        usage: Option<Usage>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| {
            let model = model.clone();
            async move {
                loop {
                    if let Some(item) = buffer.pop_front() {
                        return Some((item, (inner, buffer, state, done)));
                    }
                    if done {
                        return None;
                    }

                    match inner.next().await {
                        Some(Ok(chunk)) => {
                            match chunk {
                                crate::contracts::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::contracts::StreamChunk::Warnings { .. } => {}
                                crate::contracts::StreamChunk::TextDelta { text } => {
                                    if !text.is_empty() {
                                        buffer.push_back(Ok(chat_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            serde_json::json!({"content": text}),
                                            None,
                                            None,
                                        )));
                                    }
                                }
                                crate::contracts::StreamChunk::ToolCallStart { id, name } => {
                                    let idx = if let Some(idx) =
                                        state.tool_call_index.get(&id).copied()
                                    {
                                        idx
                                    } else {
                                        let idx = state.tool_call_index.len();
                                        state.tool_call_index.insert(id.clone(), idx);
                                        idx
                                    };
                                    buffer.push_back(Ok(chat_chunk_bytes(
                                        &state.response_id,
                                        &model,
                                        created,
                                        serde_json::json!({
                                            "tool_calls": [{
                                                "index": idx,
                                                "id": id,
                                                "type": "function",
                                                "function": { "name": name }
                                            }]
                                        }),
                                        None,
                                        None,
                                    )));
                                }
                                crate::contracts::StreamChunk::ToolCallDelta {
                                    id,
                                    arguments_delta,
                                } => {
                                    let idx = if let Some(idx) =
                                        state.tool_call_index.get(&id).copied()
                                    {
                                        idx
                                    } else {
                                        let idx = state.tool_call_index.len();
                                        state.tool_call_index.insert(id.clone(), idx);
                                        idx
                                    };
                                    if !arguments_delta.is_empty() {
                                        buffer.push_back(Ok(chat_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            serde_json::json!({
                                                "tool_calls": [{
                                                    "index": idx,
                                                    "id": id,
                                                    "type": "function",
                                                    "function": { "arguments": arguments_delta }
                                                }]
                                            }),
                                            None,
                                            None,
                                        )));
                                    }
                                }
                                crate::contracts::StreamChunk::ReasoningDelta { text } => {
                                    if !text.is_empty() {
                                        buffer.push_back(Ok(chat_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            serde_json::json!({"reasoning_content": text}),
                                            None,
                                            None,
                                        )));
                                    }
                                }
                                crate::contracts::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::contracts::StreamChunk::Usage(usage) => {
                                    state.usage = Some(usage);
                                }
                            }
                            continue;
                        }
                        Some(Err(err)) => {
                            buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            done = true;
                            continue;
                        }
                        None => {
                            let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                            buffer.push_back(Ok(chat_chunk_bytes(
                                &state.response_id,
                                &model,
                                created,
                                serde_json::json!({}),
                                Some(finish_reason),
                                None,
                            )));
                            if include_usage {
                                if let Some(usage) =
                                    state.usage.as_ref().and_then(usage_to_chat_usage)
                                {
                                    buffer.push_back(Ok(chat_usage_chunk_bytes(
                                        &state.response_id,
                                        &model,
                                        created,
                                        usage,
                                    )));
                                }
                            }
                            buffer.push_back(Ok(Bytes::from("data: [DONE]\n\n")));
                            done = true;
                            continue;
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

pub fn stream_to_completions_sse(
    stream: StreamResult,
    fallback_id: String,
    model: String,
    created: u64,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct State {
        response_id: String,
        finish_reason: Option<FinishReason>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| {
            let model = model.clone();
            async move {
                loop {
                    if let Some(item) = buffer.pop_front() {
                        return Some((item, (inner, buffer, state, done)));
                    }
                    if done {
                        return None;
                    }

                    match inner.next().await {
                        Some(Ok(chunk)) => {
                            match chunk {
                                crate::contracts::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::contracts::StreamChunk::Warnings { .. } => {}
                                crate::contracts::StreamChunk::TextDelta { text } => {
                                    if !text.is_empty() {
                                        buffer.push_back(Ok(completion_chunk_bytes(
                                            &state.response_id,
                                            &model,
                                            created,
                                            &text,
                                            None,
                                        )));
                                    }
                                }
                                crate::contracts::StreamChunk::ToolCallStart { .. } => {}
                                crate::contracts::StreamChunk::ToolCallDelta { .. } => {}
                                crate::contracts::StreamChunk::ReasoningDelta { .. } => {}
                                crate::contracts::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::contracts::StreamChunk::Usage(_) => {}
                            }
                            continue;
                        }
                        Some(Err(err)) => {
                            buffer.push_back(Err(std::io::Error::other(err.to_string())));
                            done = true;
                            continue;
                        }
                        None => {
                            let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                            buffer.push_back(Ok(completion_chunk_bytes(
                                &state.response_id,
                                &model,
                                created,
                                "",
                                Some(finish_reason),
                            )));
                            buffer.push_back(Ok(Bytes::from("data: [DONE]\n\n")));
                            done = true;
                            continue;
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

pub fn stream_to_responses_sse(
    stream: StreamResult,
    fallback_id: String,
) -> futures_util::stream::BoxStream<'static, IoResult<Bytes>> {
    #[derive(Default)]
    struct ToolCallState {
        id: String,
        name: String,
        pending_arguments: String,
    }

    #[derive(Default)]
    struct State {
        response_id: String,
        created_sent: bool,
        tool_call_index: HashMap<String, usize>,
        tool_calls: Vec<ToolCallState>,
        finish_reason: Option<FinishReason>,
        usage: Option<Usage>,
    }

    stream::unfold(
        (
            stream,
            VecDeque::<IoResult<Bytes>>::new(),
            State {
                response_id: fallback_id,
                ..State::default()
            },
            false,
        ),
        move |(mut inner, mut buffer, mut state, mut done)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((item, (inner, buffer, state, done)));
                }
                if done {
                    return None;
                }

                match inner.next().await {
                    Some(Ok(chunk)) => {
                        if let crate::contracts::StreamChunk::ResponseId { id } = &chunk {
                            let id = id.trim();
                            if !id.is_empty() {
                                state.response_id = id.to_string();
                            }
                        }

                        if !state.created_sent {
                            let response_id = state.response_id.clone();
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.created",
                                "response": { "id": response_id }
                            }))));
                            state.created_sent = true;
                        }

                        match chunk {
                            crate::contracts::StreamChunk::Warnings { .. } => {}
                            crate::contracts::StreamChunk::ResponseId { .. } => {}
                            crate::contracts::StreamChunk::TextDelta { text } => {
                                if !text.is_empty() {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.output_text.delta",
                                        "delta": text,
                                    }))));
                                }
                            }
                            crate::contracts::StreamChunk::ToolCallStart { id, name } => {
                                let idx = state
                                    .tool_call_index
                                    .entry(id.clone())
                                    .or_insert_with(|| state.tool_calls.len())
                                    .to_owned();
                                if state.tool_calls.len() <= idx {
                                    state
                                        .tool_calls
                                        .resize_with(idx.saturating_add(1), ToolCallState::default);
                                }
                                let slot = &mut state.tool_calls[idx];
                                slot.id = id;
                                slot.name = name;
                            }
                            crate::contracts::StreamChunk::ToolCallDelta {
                                id,
                                arguments_delta,
                            } => {
                                let idx = state
                                    .tool_call_index
                                    .entry(id.clone())
                                    .or_insert_with(|| state.tool_calls.len())
                                    .to_owned();
                                if state.tool_calls.len() <= idx {
                                    state
                                        .tool_calls
                                        .resize_with(idx.saturating_add(1), ToolCallState::default);
                                }
                                let slot = &mut state.tool_calls[idx];
                                if slot.id.is_empty() {
                                    slot.id = id;
                                }
                                slot.pending_arguments.push_str(&arguments_delta);
                            }
                            crate::contracts::StreamChunk::ReasoningDelta { text } => {
                                if !text.is_empty() {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.reasoning_text.delta",
                                        "delta": text,
                                    }))));
                                }
                            }
                            crate::contracts::StreamChunk::FinishReason(reason) => {
                                state.finish_reason = Some(reason);
                            }
                            crate::contracts::StreamChunk::Usage(usage) => {
                                state.usage = Some(usage);
                            }
                        }
                        continue;
                    }
                    Some(Err(err)) => {
                        buffer.push_back(Err(std::io::Error::other(err.to_string())));
                        done = true;
                        continue;
                    }
                    None => {
                        if !state.created_sent {
                            let response_id = state.response_id.clone();
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.created",
                                "response": { "id": response_id }
                            }))));
                            state.created_sent = true;
                        }

                        for (idx, slot) in state.tool_calls.iter().enumerate() {
                            let call_id = slot.id.trim();
                            let call_id = if call_id.is_empty() {
                                format!("call_{idx}")
                            } else {
                                call_id.to_string()
                            };
                            let name = slot.name.trim();
                            let name = if name.is_empty() {
                                "unknown".to_string()
                            } else {
                                name.to_string()
                            };
                            let args = slot.pending_arguments.trim();
                            if args.is_empty() && name == "unknown" {
                                continue;
                            }
                            buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                "type": "response.output_item.done",
                                "item": {
                                    "type": "function_call",
                                    "call_id": call_id,
                                    "name": name,
                                    "arguments": if args.is_empty() { "{}" } else { args },
                                }
                            }))));
                        }

                        let finish_reason = state.finish_reason.unwrap_or(FinishReason::Stop);
                        let (status, incomplete_details) =
                            finish_reason_to_responses_status(finish_reason);

                        let mut response = Map::<String, Value>::new();
                        response.insert("id".to_string(), Value::String(state.response_id.clone()));
                        response.insert("status".to_string(), Value::String(status.to_string()));
                        if let Some(incomplete_details) = incomplete_details {
                            response.insert("incomplete_details".to_string(), incomplete_details);
                        }
                        if let Some(usage) = state.usage.as_ref().and_then(usage_to_responses_usage)
                        {
                            response.insert("usage".to_string(), usage);
                        }

                        let event_kind = if status == "completed" {
                            "response.completed"
                        } else {
                            "response.incomplete"
                        };
                        buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                            "type": event_kind,
                            "response": response,
                        }))));

                        done = true;
                        continue;
                    }
                }
            }
        },
    )
    .boxed()
}

#[cfg(test)]
mod openai_protocol_reasoning_tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn chat_completions_sse_emits_reasoning_content_delta()
    -> Result<(), Box<dyn std::error::Error>> {
        let inner: StreamResult = Box::pin(futures_util::stream::iter(vec![
            Ok(crate::contracts::StreamChunk::ResponseId {
                id: "resp_1".to_string(),
            }),
            Ok(crate::contracts::StreamChunk::ReasoningDelta {
                text: "thinking...".to_string(),
            }),
            Ok(crate::contracts::StreamChunk::TextDelta {
                text: "OK".to_string(),
            }),
            Ok(crate::contracts::StreamChunk::FinishReason(
                FinishReason::Stop,
            )),
        ]));

        let mut out = Vec::<u8>::new();
        let mut s = stream_to_chat_completions_sse(
            inner,
            "fallback".to_string(),
            "stub".to_string(),
            0,
            false,
        );
        while let Some(item) = s.next().await {
            out.extend_from_slice(&item?);
        }
        let text = String::from_utf8(out)?;
        assert!(text.contains("\"reasoning_content\":\"thinking...\""));
        Ok(())
    }

    #[tokio::test]
    async fn responses_sse_emits_reasoning_text_delta_event()
    -> Result<(), Box<dyn std::error::Error>> {
        let inner: StreamResult = Box::pin(futures_util::stream::iter(vec![
            Ok(crate::contracts::StreamChunk::ResponseId {
                id: "resp_1".to_string(),
            }),
            Ok(crate::contracts::StreamChunk::ReasoningDelta {
                text: "thinking...".to_string(),
            }),
            Ok(crate::contracts::StreamChunk::FinishReason(
                FinishReason::Stop,
            )),
        ]));

        let mut out = Vec::<u8>::new();
        let mut s = stream_to_responses_sse(inner, "fallback".to_string());
        while let Some(item) = s.next().await {
            out.extend_from_slice(&item?);
        }
        let text = String::from_utf8(out)?;
        assert!(text.contains("\"type\":\"response.reasoning_text.delta\""));
        assert!(text.contains("\"delta\":\"thinking...\""));
        Ok(())
    }
}

pub fn provider_response_id(response: &GenerateResponse, fallback: &str) -> String {
    response
        .provider_metadata
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| id.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn provider_response_id_from_chunk(chunk: &crate::contracts::StreamChunk) -> Option<String> {
    match chunk {
        crate::contracts::StreamChunk::ResponseId { id } => {
            let id = id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }
        }
        _ => None,
    }
}

pub fn map_provider_error_to_openai(
    err: crate::error::DittoError,
) -> (StatusCode, &'static str, Option<&'static str>, String) {
    match err {
        crate::error::DittoError::Api { status, body } => {
            let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, "api_error", Some("provider_error"), body)
        }
        crate::error::DittoError::InvalidResponse(message)
            if message.code() == "error_detail.provider.model_missing" =>
        {
            (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                None,
                message.to_string(),
            )
        }
        crate::error::DittoError::InvalidResponse(message) => (
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            Some("unsupported_feature"),
            message.to_string(),
        ),
        other => (
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("provider_error"),
            other.to_string(),
        ),
    }
}

// end inline: ../../translation/openai_protocol.rs

#[cfg(test)]
mod error_mapping_tests {
    use super::map_provider_error_to_openai;
    use axum::http::StatusCode;

    #[test]
    fn maps_provider_model_missing_as_bad_request() {
        let (status, kind, code, message) =
            map_provider_error_to_openai(crate::error::DittoError::provider_model_missing(
                "openai",
                "set request.model or OpenAI::with_model",
            ));

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(kind, "invalid_request_error");
        assert_eq!(code, None);
        assert!(message.contains("model is not set"));
    }

    #[test]
    fn maps_provider_config_errors_as_provider_errors() {
        let (status, kind, code, message) =
            map_provider_error_to_openai(crate::error::DittoError::provider_auth_missing("vertex"));

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(kind, "api_error");
        assert_eq!(code, Some("provider_error"));
        assert!(message.contains("config error"));
    }
}
