// Gateway translation application implementation.
// inlined from ../../translation/backend.rs
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use axum::http::StatusCode;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, OnceCell};

use crate::audio::{AudioTranscriptionModel, SpeechModel};
use crate::batch::BatchClient;
use crate::embedding::EmbeddingModel;
use crate::file::{FileClient, FileContent, FileUploadRequest};
use crate::image::ImageGenerationModel;
use crate::image_edit::ImageEditModel;
use crate::model::{LanguageModel, StreamResult};
use crate::moderation::ModerationModel;
use crate::object::{LanguageModelObjectExt, ObjectOptions, ObjectOutput};
use crate::rerank::RerankModel;
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, ContentPart, FinishReason, GenerateRequest, GenerateResponse,
    ImageEditRequest, ImageEditResponse, ImageEditUpload, ImageGenerationRequest,
    ImageGenerationResponse, ImageResponseFormat, ImageSource, JsonSchemaFormat, Message,
    ModerationInput, ModerationRequest, ModerationResponse, ProviderOptions, ReasoningEffort,
    RerankRequest, RerankResponse, ResponseFormat, Role, SpeechRequest, SpeechResponse,
    SpeechResponseFormat, Tool, ToolChoice, TranscriptionResponseFormat, Usage,
    VideoContentVariant, VideoDeleteResponse, VideoGenerationRequest, VideoGenerationResponse,
    VideoListOrder, VideoListRequest, VideoListResponse, VideoRemixRequest,
};
use crate::video::VideoGenerationModel;
use crate::{DittoError, Env, ProviderConfig};

type ParseResult<T> = std::result::Result<T, String>;
type IoResult<T> = std::result::Result<T, std::io::Error>;

const DEFAULT_TRANSLATION_MODEL_CACHE_MAX_ENTRIES: usize = 64;
const DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES: usize = 128;
const MAX_TRANSLATION_MODEL_CACHE_KEY_BYTES: usize = 256;

#[derive(Debug)]
struct ModelCache<V> {
    entries: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V> Default for ModelCache<V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V: Clone> ModelCache<V> {
    fn move_key_to_back(&mut self, key: &str) {
        if self.order.back().is_some_and(|candidate| candidate == key) {
            return;
        }
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            if let Some(existing) = self.order.remove(index) {
                self.order.push_back(existing);
            }
            return;
        }
        self.order.push_back(key.to_string());
    }

    fn get(&mut self, key: &str) -> Option<V> {
        let value = self.entries.get(key).cloned()?;
        self.move_key_to_back(key);
        Some(value)
    }

    fn insert(&mut self, key: String, value: V, max_entries: usize) {
        if max_entries == 0 {
            return;
        }

        let replaced = self.entries.insert(key.clone(), value).is_some();
        if replaced {
            self.move_key_to_back(&key);
        } else {
            self.order.push_back(key);
        }

        while self.entries.len() > max_entries {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&candidate);
        }
    }

    fn remove(&mut self, key: &str) -> Option<V> {
        let value = self.entries.remove(key)?;
        if let Some(index) = self.order.iter().position(|candidate| candidate == key) {
            let _ = self.order.remove(index);
        }
        Some(value)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StoredTranslationResponse {
    pub(crate) response: Value,
    pub(crate) input_items: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::ModelCache;

    #[test]
    fn model_cache_get_promotes_recency() {
        let mut cache = ModelCache::default();
        cache.insert("a".to_string(), 1, 2);
        cache.insert("b".to_string(), 2, 2);

        assert_eq!(cache.get("a"), Some(1));
        cache.insert("c".to_string(), 3, 2);

        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c"), Some(3));
    }

    #[test]
    fn model_cache_hot_get_does_not_grow_order() {
        let mut cache = ModelCache::default();
        cache.insert("a".to_string(), 1, 10);

        for _ in 0..5 {
            assert_eq!(cache.get("a"), Some(1));
        }

        assert_eq!(cache.order.len(), 1);
        assert_eq!(cache.order.front().map(String::as_str), Some("a"));
    }
}

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
    embedding_cache: Arc<Mutex<ModelCache<Arc<dyn EmbeddingModel>>>>,
    moderation_cache: Arc<OnceCell<Arc<dyn ModerationModel>>>,
    image_generation_cache: Arc<OnceCell<Arc<dyn ImageGenerationModel>>>,
    image_edit_cache: Arc<OnceCell<Arc<dyn ImageEditModel>>>,
    video_generation_cache: Arc<OnceCell<Arc<dyn VideoGenerationModel>>>,
    audio_transcription_cache: Arc<Mutex<ModelCache<Arc<dyn AudioTranscriptionModel>>>>,
    speech_cache: Arc<Mutex<ModelCache<Arc<dyn SpeechModel>>>>,
    rerank_cache: Arc<Mutex<ModelCache<Arc<dyn RerankModel>>>>,
    batch_cache: Arc<OnceCell<Arc<dyn BatchClient>>>,
    file_cache: Arc<OnceCell<Arc<dyn FileClient>>>,
    response_store: Arc<Mutex<ModelCache<StoredTranslationResponse>>>,
}

impl Default for TranslationBackendRuntime {
    fn default() -> Self {
        Self {
            model_cache_max_entries: DEFAULT_TRANSLATION_MODEL_CACHE_MAX_ENTRIES,
            env: Env::default(),
            provider_config: ProviderConfig::default(),
            embedding_cache: Arc::new(Mutex::new(ModelCache::default())),
            moderation_cache: Arc::new(OnceCell::new()),
            image_generation_cache: Arc::new(OnceCell::new()),
            image_edit_cache: Arc::new(OnceCell::new()),
            video_generation_cache: Arc::new(OnceCell::new()),
            audio_transcription_cache: Arc::new(Mutex::new(ModelCache::default())),
            speech_cache: Arc::new(Mutex::new(ModelCache::default())),
            rerank_cache: Arc::new(Mutex::new(ModelCache::default())),
            batch_cache: Arc::new(OnceCell::new()),
            file_cache: Arc::new(OnceCell::new()),
            response_store: Arc::new(Mutex::new(ModelCache::default())),
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
        operation: crate::OperationKind,
        capability: Option<crate::CapabilityKind>,
    ) -> bool {
        let provider = provider.trim();
        if provider.is_empty() {
            return false;
        }

        let mut request = crate::RuntimeRouteRequest::new(provider, model, operation)
            .with_provider_config(&self.provider_config);
        if let Some(capability) = capability {
            request = request.with_required_capability(capability);
        }

        crate::builtin_registry()
            .resolve_runtime_route(request)
            .is_ok()
    }

    fn supports_runtime_capability(
        &self,
        provider: &str,
        model: Option<&str>,
        capability: crate::CapabilityKind,
    ) -> bool {
        let provider = provider.trim();
        if provider.is_empty() {
            return false;
        }

        let Some(plugin) = crate::builtin_registry()
            .plugin_for_runtime_request(provider, self.provider_config.runtime_hints())
        else {
            return false;
        };

        let requested_model = if capability == crate::CapabilityKind::BATCH {
            None
        } else {
            model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| self.configured_default_model())
        };

        plugin
            .capability_resolution(requested_model)
            .effective_supports(capability)
    }

    fn supports_file_builder(&self, provider: &str) -> bool {
        matches!(
            crate::builtin_registry()
                .plugin_for_runtime_request(provider.trim(), self.provider_config.runtime_hints())
                .map(|plugin| plugin.id),
            Some("openai" | "openai-compatible")
        )
    }

    async fn resolve_embedding_model(
        &self,
        provider: &str,
        direct: Option<&Arc<dyn EmbeddingModel>>,
        model: &str,
    ) -> crate::Result<Arc<dyn EmbeddingModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "embedding model is missing".to_string(),
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
                DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn ModerationModel>> {
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
                        DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn ImageGenerationModel>> {
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
                        DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn ImageEditModel>> {
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
                        DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn VideoGenerationModel>> {
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
                        DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn AudioTranscriptionModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "audio transcription model is missing".to_string(),
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
                DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn SpeechModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "speech model is missing".to_string(),
            ));
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
                DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn RerankModel>> {
        if let Some(model_impl) = direct.cloned() {
            return Ok(model_impl);
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "rerank model is missing".to_string(),
            ));
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
                DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn BatchClient>> {
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
                        DittoError::InvalidResponse(format!(
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
    ) -> crate::Result<Arc<dyn FileClient>> {
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
                        DittoError::InvalidResponse(format!(
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

pub(crate) async fn delete_stored_response_from_translation_backends(
    backends: &HashMap<String, TranslationBackend>,
    response_id: &str,
) -> Option<(String, String)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let mut backend_names = backends.keys().cloned().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let Some(backend) = backends.get(&backend_name) else {
            continue;
        };
        if !backend.delete_stored_response(response_id).await {
            continue;
        }
        let provider = backend.provider_name().trim();
        let provider = if provider.is_empty() {
            backend_name.clone()
        } else {
            provider.to_string()
        };
        return Some((backend_name, provider));
    }

    None
}

pub(crate) async fn find_stored_response_from_translation_backends(
    backends: &HashMap<String, TranslationBackend>,
    response_id: &str,
) -> Option<(String, String, StoredTranslationResponse)> {
    let response_id = response_id.trim();
    if response_id.is_empty() {
        return None;
    }

    let mut backend_names = backends.keys().cloned().collect::<Vec<_>>();
    backend_names.sort();

    for backend_name in backend_names {
        let Some(backend) = backends.get(&backend_name) else {
            continue;
        };
        let Some(stored) = backend.stored_response(response_id).await else {
            continue;
        };
        let provider = backend.provider_name().trim();
        let provider = if provider.is_empty() {
            backend_name.clone()
        } else {
            provider.to_string()
        };
        return Some((backend_name, provider, stored));
    }

    None
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

    fn bound_supports_capability(&self, capability: crate::CapabilityKind) -> bool {
        match capability {
            crate::CapabilityKind::LLM => true,
            crate::CapabilityKind::EMBEDDING => self.bindings.embedding_model.is_some(),
            crate::CapabilityKind::IMAGE_GENERATION => {
                self.bindings.image_generation_model.is_some()
            }
            crate::CapabilityKind::IMAGE_EDIT => self.bindings.image_edit_model.is_some(),
            crate::CapabilityKind::VIDEO_GENERATION => {
                self.bindings.video_generation_model.is_some()
            }
            crate::CapabilityKind::MODERATION => self.bindings.moderation_model.is_some(),
            crate::CapabilityKind::AUDIO_TRANSCRIPTION
            | crate::CapabilityKind::AUDIO_TRANSLATION => {
                self.bindings.audio_transcription_model.is_some()
            }
            crate::CapabilityKind::AUDIO_SPEECH => self.bindings.speech_model.is_some(),
            crate::CapabilityKind::RERANK => self.bindings.rerank_model.is_some(),
            crate::CapabilityKind::BATCH => self.bindings.batch_client.is_some(),
            _ => false,
        }
    }

    fn bound_supports_runtime_operation(&self, operation: crate::OperationKind) -> bool {
        match operation {
            crate::OperationKind::CHAT_COMPLETION
            | crate::OperationKind::RESPONSE
            | crate::OperationKind::TEXT_COMPLETION
            | crate::OperationKind::THREAD_RUN
            | crate::OperationKind::GROUP_CHAT_COMPLETION
            | crate::OperationKind::CHAT_TRANSLATION => true,
            crate::OperationKind::EMBEDDING | crate::OperationKind::MULTIMODAL_EMBEDDING => {
                self.bindings.embedding_model.is_some()
            }
            crate::OperationKind::IMAGE_GENERATION => {
                self.bindings.image_generation_model.is_some()
            }
            crate::OperationKind::IMAGE_EDIT => self.bindings.image_edit_model.is_some(),
            crate::OperationKind::VIDEO_GENERATION => {
                self.bindings.video_generation_model.is_some()
            }
            crate::OperationKind::MODERATION => self.bindings.moderation_model.is_some(),
            crate::OperationKind::AUDIO_TRANSCRIPTION | crate::OperationKind::AUDIO_TRANSLATION => {
                self.bindings.audio_transcription_model.is_some()
            }
            crate::OperationKind::AUDIO_SPEECH => self.bindings.speech_model.is_some(),
            crate::OperationKind::RERANK => self.bindings.rerank_model.is_some(),
            crate::OperationKind::BATCH => self.bindings.batch_client.is_some(),
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
        operation: Option<crate::OperationKind>,
        capabilities: &'static [crate::CapabilityKind],
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

    pub(crate) async fn store_response_record(
        &self,
        response_id: &str,
        response: Value,
        input_items: Vec<Value>,
    ) {
        let response_id = response_id.trim();
        if response_id.is_empty() {
            return;
        }

        let mut store = self.runtime.response_store.lock().await;
        store.insert(
            response_id.to_string(),
            StoredTranslationResponse {
                response,
                input_items,
            },
            DEFAULT_TRANSLATION_RESPONSE_STORE_MAX_ENTRIES,
        );
    }

    async fn stored_response(&self, response_id: &str) -> Option<StoredTranslationResponse> {
        self.runtime
            .response_store
            .lock()
            .await
            .get(response_id.trim())
    }

    async fn delete_stored_response(&self, response_id: &str) -> bool {
        self.runtime
            .response_store
            .lock()
            .await
            .remove(response_id.trim())
            .is_some()
    }

    pub async fn upload_file(&self, request: FileUploadRequest) -> crate::Result<String> {
        let client = self.resolve_file_client().await?;
        client.upload_file_with_purpose(request).await
    }

    pub async fn embed(&self, model: &str, texts: Vec<String>) -> crate::Result<Vec<Vec<f32>>> {
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

    pub async fn moderate(&self, request: ModerationRequest) -> crate::Result<ModerationResponse> {
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
    ) -> crate::Result<ImageGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_image_generation_model(
                self.provider_name(),
                self.bindings.image_generation_model.as_ref(),
            )
            .await?;

        model_impl.generate(request).await
    }

    pub async fn edit_image(&self, request: ImageEditRequest) -> crate::Result<ImageEditResponse> {
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
    ) -> crate::Result<VideoGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.create(request).await
    }

    pub async fn retrieve_video(&self, video_id: &str) -> crate::Result<VideoGenerationResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.retrieve(video_id).await
    }

    pub async fn list_videos(&self, request: VideoListRequest) -> crate::Result<VideoListResponse> {
        let model_impl = self
            .runtime
            .resolve_video_generation_model(
                self.provider_name(),
                self.bindings.video_generation_model.as_ref(),
            )
            .await?;

        model_impl.list(request).await
    }

    pub async fn delete_video(&self, video_id: &str) -> crate::Result<VideoDeleteResponse> {
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
    ) -> crate::Result<FileContent> {
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
    ) -> crate::Result<VideoGenerationResponse> {
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
    ) -> crate::Result<AudioTranscriptionResponse> {
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
    ) -> crate::Result<SpeechResponse> {
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
    ) -> crate::Result<RerankResponse> {
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

    pub async fn create_batch(&self, request: BatchCreateRequest) -> crate::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.create(request).await
    }

    pub async fn retrieve_batch(&self, batch_id: &str) -> crate::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.retrieve(batch_id).await
    }

    pub async fn cancel_batch(&self, batch_id: &str) -> crate::Result<BatchResponse> {
        let client = self.resolve_batch_client().await?;
        client.cancel(batch_id).await
    }

    pub async fn list_batches(
        &self,
        limit: Option<u32>,
        after: Option<String>,
    ) -> crate::Result<BatchListResponse> {
        let client = self.resolve_batch_client().await?;
        client.list(limit, after).await
    }

    async fn resolve_batch_client(&self) -> crate::Result<Arc<dyn BatchClient>> {
        self.runtime
            .resolve_batch_client(self.provider_name(), self.bindings.batch_client.as_ref())
            .await
    }

    pub async fn compact_responses_history(
        &self,
        model: &str,
        instructions: &str,
        input: &[Value],
    ) -> crate::Result<(Vec<Value>, Usage)> {
        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "compaction model is missing".to_string(),
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
            return Err(DittoError::InvalidResponse(
                "compaction response is not a JSON array".to_string(),
            ));
        };

        let mut usage = out.response.usage;
        usage.merge_total();
        Ok((items, usage))
    }
}
// end inline: ../../translation/backend.rs
// inlined from ../../translation/model_builders.rs
pub use crate::runtime::model_builders::{
    build_audio_transcription_model, build_batch_client, build_context_cache_model,
    build_embedding_model, build_file_client, build_image_edit_model, build_image_generation_model,
    build_language_model, build_moderation_model, build_realtime_session_model, build_rerank_model,
    build_speech_model, build_video_generation_model,
};
// end inline: ../../translation/model_builders.rs
// inlined from ../../translation/openai_endpoints.rs
pub fn is_chat_completions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/chat/completions" || path == "/v1/chat/completions/"
}

pub fn is_completions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/completions" || path == "/v1/completions/"
}

pub fn is_models_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/models" || path == "/v1/models/"
}

pub fn models_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/models/")?;
    if rest.trim().is_empty() {
        return None;
    }
    Some(rest.to_string())
}

pub fn is_responses_create_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses" || path == "/v1/responses/"
}

pub fn is_responses_compact_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses/compact" || path == "/v1/responses/compact/"
}

pub fn is_responses_input_tokens_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/responses/input_tokens" || path == "/v1/responses/input_tokens/"
}

pub fn is_embeddings_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/embeddings" || path == "/v1/embeddings/"
}

pub fn is_moderations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/moderations" || path == "/v1/moderations/"
}

pub fn is_images_generations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/images/generations" || path == "/v1/images/generations/"
}

pub fn is_images_edits_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/images/edits" || path == "/v1/images/edits/"
}

pub fn is_audio_transcriptions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/transcriptions" || path == "/v1/audio/transcriptions/"
}

pub fn is_audio_translations_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/translations" || path == "/v1/audio/translations/"
}

pub fn is_videos_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/videos" || path == "/v1/videos/"
}

pub fn is_audio_speech_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/speech" || path == "/v1/audio/speech/"
}

pub fn is_batches_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/batches" || path == "/v1/batches/"
}

pub fn is_rerank_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/rerank" || path == "/v1/rerank/"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationEndpointKind {
    ChatCompletions,
    Completions,
    ResponsesCreate,
    ResponsesCompact,
    ResponsesInputTokens,
    ResponsesRetrieve,
    ResponsesInputItems,
    Embeddings,
    Moderations,
    ImagesGenerations,
    ImagesEdits,
    AudioTranscriptions,
    AudioTranslations,
    VideosRoot,
    VideoRetrieve,
    VideoContent,
    VideoRemix,
    AudioSpeech,
    BatchesRoot,
    BatchRetrieve,
    BatchCancel,
    Rerank,
    ModelsList,
    ModelsRetrieve,
    FilesRoot,
    FilesRetrieve,
    FilesContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationEndpointRequirement {
    None,
    RuntimeCapability(&'static [crate::CapabilityKind]),
    FilesApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationEndpointDescriptor {
    pub kind: TranslationEndpointKind,
    pub runtime_operation: Option<crate::OperationKind>,
    pub requirement: TranslationEndpointRequirement,
}

const LLM_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] = &[crate::CapabilityKind::LLM];
const EMBEDDING_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::EMBEDDING];
const MODERATION_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::MODERATION];
const IMAGE_GENERATION_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::IMAGE_GENERATION];
const IMAGE_EDIT_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::IMAGE_EDIT];
const AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::AUDIO_TRANSCRIPTION];
const VIDEO_GENERATION_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::VIDEO_GENERATION];
const AUDIO_SPEECH_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] =
    &[crate::CapabilityKind::AUDIO_SPEECH];
const RERANK_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] = &[crate::CapabilityKind::RERANK];
const BATCH_RUNTIME_CAPABILITIES: &[crate::CapabilityKind] = &[crate::CapabilityKind::BATCH];

pub fn translation_endpoint_descriptor(
    method: &axum::http::Method,
    path_and_query: &str,
) -> Option<TranslationEndpointDescriptor> {
    use axum::http::Method;

    if *method == Method::POST {
        if is_chat_completions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ChatCompletions,
                runtime_operation: Some(crate::OperationKind::CHAT_COMPLETION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_completions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Completions,
                runtime_operation: Some(crate::OperationKind::TEXT_COMPLETION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_create_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesCreate,
                runtime_operation: Some(crate::OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_compact_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesCompact,
                runtime_operation: Some(crate::OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_input_tokens_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesInputTokens,
                runtime_operation: Some(crate::OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_embeddings_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Embeddings,
                runtime_operation: Some(crate::OperationKind::EMBEDDING),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    EMBEDDING_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_moderations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Moderations,
                runtime_operation: Some(crate::OperationKind::MODERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    MODERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_images_generations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ImagesGenerations,
                runtime_operation: Some(crate::OperationKind::IMAGE_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    IMAGE_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_images_edits_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ImagesEdits,
                runtime_operation: Some(crate::OperationKind::IMAGE_EDIT),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    IMAGE_EDIT_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_transcriptions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioTranscriptions,
                runtime_operation: Some(crate::OperationKind::AUDIO_TRANSCRIPTION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_translations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioTranslations,
                runtime_operation: Some(crate::OperationKind::AUDIO_TRANSCRIPTION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_videos_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideosRoot,
                runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_remix_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoRemix,
                runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_speech_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioSpeech,
                runtime_operation: Some(crate::OperationKind::AUDIO_SPEECH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_SPEECH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_rerank_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Rerank,
                runtime_operation: Some(crate::OperationKind::RERANK),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    RERANK_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_batches_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchesRoot,
                runtime_operation: Some(crate::OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if batches_cancel_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchCancel,
                runtime_operation: Some(crate::OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_files_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRoot,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
    } else if *method == Method::GET {
        if is_batches_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchesRoot,
                runtime_operation: Some(crate::OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if responses_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if responses_input_items_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesInputItems,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if batches_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchRetrieve,
                runtime_operation: Some(crate::OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_videos_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideosRoot,
                runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoRetrieve,
                runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_content_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoContent,
                runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_models_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ModelsList,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if models_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ModelsRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if is_files_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRoot,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
        if files_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
        if files_content_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesContent,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
    } else if *method == Method::DELETE && videos_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::VideoRetrieve,
            runtime_operation: Some(crate::OperationKind::VIDEO_GENERATION),
            requirement: TranslationEndpointRequirement::RuntimeCapability(
                VIDEO_GENERATION_RUNTIME_CAPABILITIES,
            ),
        });
    } else if *method == Method::DELETE && responses_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::ResponsesRetrieve,
            runtime_operation: None,
            requirement: TranslationEndpointRequirement::None,
        });
    } else if *method == Method::DELETE && files_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::FilesRetrieve,
            runtime_operation: None,
            requirement: TranslationEndpointRequirement::FilesApi,
        });
    }

    None
}

fn responses_subresource_id(path_and_query: &str, suffix: Option<&str>) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/responses/")?;
    if rest.trim().is_empty() || rest == "compact" || rest == "input_tokens" {
        return None;
    }

    match suffix {
        Some(suffix) => {
            let (response_id, found_suffix) = rest.split_once('/')?;
            if response_id.trim().is_empty() || found_suffix != suffix {
                return None;
            }
            Some(response_id.to_string())
        }
        None => {
            if rest.contains('/') {
                return None;
            }
            Some(rest.to_string())
        }
    }
}

pub fn responses_retrieve_id(path_and_query: &str) -> Option<String> {
    responses_subresource_id(path_and_query, None)
}

pub fn responses_input_items_id(path_and_query: &str) -> Option<String> {
    responses_subresource_id(path_and_query, Some("input_items"))
}

pub fn batches_cancel_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    let (batch_id, suffix) = rest.split_once('/')?;
    if batch_id.trim().is_empty() {
        return None;
    }
    if suffix == "cancel" {
        return Some(batch_id.to_string());
    }
    None
}

fn videos_subresource_id(path_and_query: &str, suffix: Option<&str>) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/videos/")?;
    if rest.trim().is_empty() {
        return None;
    }

    match suffix {
        Some(suffix) => {
            let (video_id, found_suffix) = rest.split_once('/')?;
            if video_id.trim().is_empty() || found_suffix != suffix {
                return None;
            }
            Some(video_id.to_string())
        }
        None => {
            if rest.contains('/') {
                return None;
            }
            Some(rest.to_string())
        }
    }
}

pub fn videos_retrieve_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, None)
}

pub fn videos_content_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, Some("content"))
}

pub fn videos_remix_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, Some("remix"))
}

pub fn batches_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

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
    let parts = super::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        if part.name != field_name {
            continue;
        }
        if part.filename.is_some() {
            continue;
        }
        let text = String::from_utf8_lossy(part.data.as_ref())
            .trim()
            .to_string();
        if text.is_empty() {
            return Ok(None);
        }
        return Ok(Some(text));
    }
    Ok(None)
}

pub fn audio_transcriptions_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<AudioTranscriptionRequest> {
    let mut file: Option<super::multipart::MultipartPart> = None;
    let mut model: Option<String> = None;
    let mut language: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<TranscriptionResponseFormat> = None;
    let mut temperature: Option<f32> = None;

    let parts = super::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "file" => {
                file = Some(part);
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "language" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    language = Some(value);
                }
            }
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "response_format" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                response_format = match value.as_str() {
                    "json" => Some(TranscriptionResponseFormat::Json),
                    "text" => Some(TranscriptionResponseFormat::Text),
                    "srt" => Some(TranscriptionResponseFormat::Srt),
                    "verbose_json" => Some(TranscriptionResponseFormat::VerboseJson),
                    "vtt" => Some(TranscriptionResponseFormat::Vtt),
                    _ => None,
                };
            }
            "temperature" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if let Ok(parsed) = value.parse::<f32>() {
                    if parsed.is_finite() {
                        temperature = Some(parsed);
                    }
                }
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| "audio/transcriptions request missing file".to_string())?;
    let model = model.ok_or_else(|| "audio/transcriptions request missing model".to_string())?;

    let filename = file
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "audio".to_string());

    Ok(AudioTranscriptionRequest {
        audio: file.data.to_vec(),
        filename,
        media_type: file.content_type.clone(),
        model: Some(model),
        language,
        prompt,
        response_format,
        temperature,
        provider_options: None,
    })
}

pub fn audio_speech_request_to_request(request: &Value) -> ParseResult<SpeechRequest> {
    serde_json::from_value::<SpeechRequest>(request.clone())
        .map_err(|err| format!("audio/speech request is invalid: {err}"))
}

pub fn speech_response_format_to_content_type(
    format: Option<SpeechResponseFormat>,
) -> &'static str {
    match format {
        Some(SpeechResponseFormat::Mp3) => "audio/mpeg",
        Some(SpeechResponseFormat::Opus) => "audio/opus",
        Some(SpeechResponseFormat::Aac) => "audio/aac",
        Some(SpeechResponseFormat::Flac) => "audio/flac",
        Some(SpeechResponseFormat::Wav) => "audio/wav",
        Some(SpeechResponseFormat::Pcm) => "audio/pcm",
        None => "application/octet-stream",
    }
}

pub fn transcription_format_to_content_type(
    format: Option<TranscriptionResponseFormat>,
) -> (&'static str, bool) {
    match format {
        Some(TranscriptionResponseFormat::Text) => ("text/plain; charset=utf-8", false),
        Some(TranscriptionResponseFormat::Srt) => ("application/x-subrip", false),
        Some(TranscriptionResponseFormat::Vtt) => ("text/vtt", false),
        Some(TranscriptionResponseFormat::Json) => ("application/json", true),
        Some(TranscriptionResponseFormat::VerboseJson) => ("application/json", true),
        None => ("application/json", true),
    }
}

pub fn chat_completions_request_to_generate_request(
    request: &Value,
) -> ParseResult<GenerateRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "chat/completions request must be a JSON object".to_string())?;

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat/completions request missing model".to_string())?;

    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "chat/completions request missing messages".to_string())?;

    let mut out_messages = Vec::<Message>::new();
    for msg in messages {
        out_messages.push(parse_openai_chat_message(msg)?);
    }

    let mut out: GenerateRequest = out_messages.into();
    out.model = Some(model.to_string());

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64) {
        if temperature.is_finite() {
            out.temperature = Some(temperature as f32);
        }
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64) {
        if top_p.is_finite() {
            out.top_p = Some(top_p as f32);
        }
    }
    if let Some(seed) = obj.get("seed").and_then(Value::as_u64) {
        out.seed = Some(seed);
    }
    if let Some(presence_penalty) = obj.get("presence_penalty").and_then(Value::as_f64) {
        if presence_penalty.is_finite() {
            out.presence_penalty = Some(presence_penalty as f32);
        }
    }
    if let Some(frequency_penalty) = obj.get("frequency_penalty").and_then(Value::as_f64) {
        if frequency_penalty.is_finite() {
            out.frequency_penalty = Some(frequency_penalty as f32);
        }
    }
    if let Some(user) = obj
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.user = Some(user.to_string());
    }
    if let Some(logprobs) = obj.get("logprobs").and_then(Value::as_bool) {
        out.logprobs = Some(logprobs);
    }
    if let Some(top_logprobs) = obj.get("top_logprobs").and_then(Value::as_u64) {
        out.top_logprobs = Some(top_logprobs.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    if let Some(tools_value) = obj.get("tools") {
        out.tools = Some(parse_openai_tools(tools_value)?);
    }
    if let Some(tool_choice_value) = obj.get("tool_choice") {
        out.tool_choice = parse_openai_tool_choice(tool_choice_value)?;
    }

    let provider_options = parse_provider_options_from_openai_request(obj);
    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            crate::types::ProviderOptionsEnvelope::from_options(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(out)
}

pub fn completions_request_to_generate_request(request: &Value) -> ParseResult<GenerateRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "completions request must be a JSON object".to_string())?;

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "completions request missing model".to_string())?;

    if let Some(suffix) = obj
        .get("suffix")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|suffix| !suffix.is_empty())
    {
        return Err(format!("unsupported completions suffix: {suffix}"));
    }

    let prompt = match obj.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.to_string(),
        Some(Value::Array(items)) => {
            if items.len() > 1 {
                return Err("completions prompt arrays are not supported".to_string());
            }
            items
                .first()
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_default()
        }
        _ => return Err("completions prompt must be a string".to_string()),
    };

    let mut out: GenerateRequest = vec![Message::user(prompt)].into();
    out.model = Some(model.to_string());

    if let Some(temperature) = obj.get("temperature").and_then(Value::as_f64) {
        if temperature.is_finite() {
            out.temperature = Some(temperature as f32);
        }
    }
    if let Some(top_p) = obj.get("top_p").and_then(Value::as_f64) {
        if top_p.is_finite() {
            out.top_p = Some(top_p as f32);
        }
    }
    if let Some(seed) = obj.get("seed").and_then(Value::as_u64) {
        out.seed = Some(seed);
    }
    if let Some(presence_penalty) = obj.get("presence_penalty").and_then(Value::as_f64) {
        if presence_penalty.is_finite() {
            out.presence_penalty = Some(presence_penalty as f32);
        }
    }
    if let Some(frequency_penalty) = obj.get("frequency_penalty").and_then(Value::as_f64) {
        if frequency_penalty.is_finite() {
            out.frequency_penalty = Some(frequency_penalty as f32);
        }
    }
    if let Some(user) = obj
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.user = Some(user.to_string());
    }
    if let Some(logprobs) = obj.get("logprobs").and_then(Value::as_u64) {
        if logprobs > 0 {
            out.logprobs = Some(true);
            out.top_logprobs = Some(logprobs.min(u64::from(u32::MAX)) as u32);
        }
    }
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    let provider_options = parse_provider_options_from_openai_request(obj);
    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            crate::types::ProviderOptionsEnvelope::from_options(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

    Ok(out)
}

pub fn embeddings_request_to_texts(request: &Value) -> ParseResult<Vec<String>> {
    let obj = request
        .as_object()
        .ok_or_else(|| "embeddings request must be a JSON object".to_string())?;

    if let Some(format) = obj.get("encoding_format").and_then(Value::as_str) {
        let format = format.trim();
        if !format.is_empty() && format != "float" {
            return Err(format!("unsupported encoding_format: {format}"));
        }
    }

    let input = obj
        .get("input")
        .ok_or_else(|| "embeddings request missing input".to_string())?;

    match input {
        Value::String(text) => Ok(vec![text.clone()]),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                match item {
                    Value::String(text) => out.push(text.clone()),
                    _ => return Err(format!("embeddings input[{idx}] must be a string")),
                }
            }
            if out.is_empty() {
                return Err("embeddings request input must not be empty".to_string());
            }
            Ok(out)
        }
        _ => Err("embeddings request input must be a string or array of strings".to_string()),
    }
}

pub fn moderations_request_to_request(request: &Value) -> ParseResult<ModerationRequest> {
    let obj = request
        .as_object()
        .ok_or_else(|| "moderations request must be a JSON object".to_string())?;

    let input = obj
        .get("input")
        .ok_or_else(|| "moderations request missing input".to_string())?;

    let input = match input {
        Value::String(text) => ModerationInput::Text(text.clone()),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let text = item
                    .as_str()
                    .ok_or_else(|| format!("moderations input[{idx}] must be a string"))?;
                out.push(text.to_string());
            }
            ModerationInput::TextArray(out)
        }
        other => ModerationInput::Raw(other.clone()),
    };

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    Ok(ModerationRequest {
        input,
        model,
        provider_options: None,
    })
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
    serde_json::from_value::<ImageGenerationRequest>(request.clone()).map_err(|err| {
        format!("images/generations request cannot be parsed as ImageGenerationRequest: {err}")
    })
}

fn parse_image_response_format(value: &str) -> ParseResult<ImageResponseFormat> {
    match value.trim() {
        "url" => Ok(ImageResponseFormat::Url),
        "b64_json" => Ok(ImageResponseFormat::Base64Json),
        other => Err(format!(
            "images/edits request has unsupported response_format: {other}"
        )),
    }
}

fn image_edit_upload_from_part(
    field_name: &str,
    part: super::multipart::MultipartPart,
) -> ImageEditUpload {
    let filename = part
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| field_name.to_string());
    ImageEditUpload {
        data: part.data.to_vec(),
        filename,
        media_type: part.content_type.clone(),
    }
}

pub fn images_edits_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<ImageEditRequest> {
    let mut prompt: Option<String> = None;
    let mut images = Vec::<ImageEditUpload>::new();
    let mut mask: Option<ImageEditUpload> = None;
    let mut model: Option<String> = None;
    let mut n: Option<u32> = None;
    let mut size: Option<String> = None;
    let mut response_format: Option<ImageResponseFormat> = None;

    let parts = super::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "image" => images.push(image_edit_upload_from_part("image", part)),
            "mask" => {
                if mask.is_none() {
                    mask = Some(image_edit_upload_from_part("mask", part));
                }
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "n" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    n = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("images/edits request has invalid n: {value}"))?,
                    );
                }
            }
            "size" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    size = Some(value);
                }
            }
            "response_format" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    response_format = Some(parse_image_response_format(&value)?);
                }
            }
            _ => {}
        }
    }

    let prompt = prompt.ok_or_else(|| "images/edits request missing prompt".to_string())?;
    if images.is_empty() {
        return Err("images/edits request missing image".to_string());
    }

    Ok(ImageEditRequest {
        prompt,
        images,
        mask,
        model,
        n,
        size,
        response_format,
        provider_options: None,
    })
}

pub fn responses_input_items_from_value(input: &Value) -> ParseResult<Vec<Value>> {
    match input {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(_) => Ok(vec![input.clone()]),
        Value::String(text) => Ok(vec![serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        })]),
        _ => Err("`input` must be a string, array, or object".to_string()),
    }
}

pub fn videos_create_request_to_request(request: &Value) -> ParseResult<VideoGenerationRequest> {
    serde_json::from_value::<VideoGenerationRequest>(request.clone())
        .map_err(|err| format!("videos request cannot be parsed as VideoGenerationRequest: {err}"))
}

pub fn videos_create_multipart_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<VideoGenerationRequest> {
    let mut prompt: Option<String> = None;
    let mut input_reference: Option<crate::types::VideoReferenceUpload> = None;
    let mut model: Option<String> = None;
    let mut seconds: Option<u32> = None;
    let mut size: Option<String> = None;

    let parts = super::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "prompt" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    prompt = Some(value);
                }
            }
            "input_reference" => {
                let filename = part
                    .filename
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "input_reference".to_string());
                input_reference = Some(crate::types::VideoReferenceUpload {
                    data: part.data.to_vec(),
                    filename,
                    media_type: part.content_type.clone(),
                });
            }
            "model" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    model = Some(value);
                }
            }
            "seconds" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    seconds = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("videos request has invalid seconds: {value}"))?,
                    );
                }
            }
            "size" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    size = Some(value);
                }
            }
            _ => {}
        }
    }

    Ok(VideoGenerationRequest {
        prompt: prompt.ok_or_else(|| "videos request missing prompt".to_string())?,
        input_reference,
        model,
        seconds,
        size,
        provider_options: None,
    })
}

pub fn videos_remix_request_to_request(request: &Value) -> ParseResult<VideoRemixRequest> {
    serde_json::from_value::<VideoRemixRequest>(request.clone())
        .map_err(|err| format!("videos remix request cannot be parsed as VideoRemixRequest: {err}"))
}

pub fn videos_content_variant_from_path(
    path_and_query: &str,
) -> ParseResult<Option<VideoContentVariant>> {
    let query = match path_and_query.split_once('?') {
        Some((_, query)) => query,
        None => return Ok(None),
    };

    let mut variant = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != "variant" || value.is_empty() {
            continue;
        }

        variant = Some(match value {
            "video" => VideoContentVariant::Video,
            "thumbnail" => VideoContentVariant::Thumbnail,
            "spritesheet" => VideoContentVariant::Spritesheet,
            _ => {
                return Err(format!(
                    "videos content request has unsupported variant: {value}"
                ));
            }
        });
    }

    Ok(variant)
}

pub fn videos_list_request_from_path(path_and_query: &str) -> ParseResult<VideoListRequest> {
    let query = match path_and_query.split_once('?') {
        Some((_, query)) => query,
        None => {
            return Ok(VideoListRequest::default());
        }
    };

    let mut request = VideoListRequest::default();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "limit" if !value.is_empty() => {
                request.limit = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("videos list request has invalid limit: {value}"))?,
                );
            }
            "after" if !value.is_empty() => {
                request.after = Some(value.to_string());
            }
            "order" if !value.is_empty() => {
                request.order = Some(match value {
                    "asc" => VideoListOrder::Asc,
                    "desc" => VideoListOrder::Desc,
                    _ => {
                        return Err(format!(
                            "videos list request has unsupported order: {value}"
                        ));
                    }
                });
            }
            _ => {}
        }
    }

    Ok(request)
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
    let chat = super::responses_shim::responses_request_to_chat_completions(request)
        .ok_or_else(|| "responses request cannot be mapped to chat/completions".to_string())?;
    let mut out = chat_completions_request_to_generate_request(&chat)?;

    let obj = request
        .as_object()
        .ok_or_else(|| "responses request must be a JSON object".to_string())?;

    let mut provider_options = ProviderOptions::default();
    if let Some(existing) = out.parsed_provider_options().ok().flatten() {
        provider_options = existing;
    }

    if let Some(reasoning) = obj.get("reasoning").and_then(Value::as_object) {
        if let Some(effort) = reasoning
            .get("effort")
            .and_then(Value::as_str)
            .and_then(parse_reasoning_effort)
        {
            provider_options.reasoning_effort = Some(effort);
        }
    }
    if let Some(parallel) = obj.get("parallel_tool_calls").and_then(Value::as_bool) {
        provider_options.parallel_tool_calls = Some(parallel);
    }
    if let Some(format_value) = obj.get("response_format").and_then(Value::as_object) {
        if let Some(parsed) = parse_json_schema_response_format(format_value) {
            provider_options.response_format = Some(parsed);
        }
    }

    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            crate::types::ProviderOptionsEnvelope::from_options(provider_options)
                .map_err(|err| format!("failed to serialize provider_options: {err}"))?,
        );
    }

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
                                crate::types::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::types::StreamChunk::Warnings { .. } => {}
                                crate::types::StreamChunk::TextDelta { text } => {
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
                                crate::types::StreamChunk::ToolCallStart { id, name } => {
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
                                crate::types::StreamChunk::ToolCallDelta {
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
                                crate::types::StreamChunk::ReasoningDelta { text } => {
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
                                crate::types::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::types::StreamChunk::Usage(usage) => {
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
                                crate::types::StreamChunk::ResponseId { id } => {
                                    let id = id.trim();
                                    if !id.is_empty() {
                                        state.response_id = id.to_string();
                                    }
                                }
                                crate::types::StreamChunk::Warnings { .. } => {}
                                crate::types::StreamChunk::TextDelta { text } => {
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
                                crate::types::StreamChunk::ToolCallStart { .. } => {}
                                crate::types::StreamChunk::ToolCallDelta { .. } => {}
                                crate::types::StreamChunk::ReasoningDelta { .. } => {}
                                crate::types::StreamChunk::FinishReason(reason) => {
                                    state.finish_reason = Some(reason);
                                }
                                crate::types::StreamChunk::Usage(_) => {}
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
                        if let crate::types::StreamChunk::ResponseId { id } = &chunk {
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
                            crate::types::StreamChunk::Warnings { .. } => {}
                            crate::types::StreamChunk::ResponseId { .. } => {}
                            crate::types::StreamChunk::TextDelta { text } => {
                                if !text.is_empty() {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.output_text.delta",
                                        "delta": text,
                                    }))));
                                }
                            }
                            crate::types::StreamChunk::ToolCallStart { id, name } => {
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
                            crate::types::StreamChunk::ToolCallDelta {
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
                            crate::types::StreamChunk::ReasoningDelta { text } => {
                                if !text.is_empty() {
                                    buffer.push_back(Ok(sse_event_bytes(serde_json::json!({
                                        "type": "response.reasoning_text.delta",
                                        "delta": text,
                                    }))));
                                }
                            }
                            crate::types::StreamChunk::FinishReason(reason) => {
                                state.finish_reason = Some(reason);
                            }
                            crate::types::StreamChunk::Usage(usage) => {
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
            Ok(crate::types::StreamChunk::ResponseId {
                id: "resp_1".to_string(),
            }),
            Ok(crate::types::StreamChunk::ReasoningDelta {
                text: "thinking...".to_string(),
            }),
            Ok(crate::types::StreamChunk::TextDelta {
                text: "OK".to_string(),
            }),
            Ok(crate::types::StreamChunk::FinishReason(FinishReason::Stop)),
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
            Ok(crate::types::StreamChunk::ResponseId {
                id: "resp_1".to_string(),
            }),
            Ok(crate::types::StreamChunk::ReasoningDelta {
                text: "thinking...".to_string(),
            }),
            Ok(crate::types::StreamChunk::FinishReason(FinishReason::Stop)),
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

pub fn provider_response_id_from_chunk(chunk: &crate::types::StreamChunk) -> Option<String> {
    match chunk {
        crate::types::StreamChunk::ResponseId { id } => {
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
    err: crate::DittoError,
) -> (StatusCode, &'static str, Option<&'static str>, String) {
    match err {
        crate::DittoError::Api { status, body } => {
            let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, "api_error", Some("provider_error"), body)
        }
        crate::DittoError::InvalidResponse(message) => (
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            Some("unsupported_feature"),
            message,
        ),
        other => (
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("provider_error"),
            other.to_string(),
        ),
    }
}

fn parse_openai_chat_message(message: &Value) -> ParseResult<Message> {
    let obj = message
        .as_object()
        .ok_or_else(|| "chat message must be an object".to_string())?;

    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat message missing role".to_string())?;

    let role = match role {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        other => return Err(format!("unsupported role: {other}")),
    };

    if role == Role::Tool {
        let tool_call_id = obj
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "tool message missing tool_call_id".to_string())?;
        let content = obj
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Ok(Message::tool_result(tool_call_id, content));
    }

    let mut parts = Vec::<ContentPart>::new();
    if let Some(content) = obj.get("content") {
        parts.extend(parse_openai_content_parts(content));
    }

    if role == Role::Assistant {
        if let Some(tool_calls) = obj.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                if let Some(part) = parse_openai_tool_call(call) {
                    parts.push(part);
                }
            }
        } else if let Some(function_call) = obj.get("function_call").and_then(Value::as_object) {
            if let Some(part) = parse_openai_function_call(function_call) {
                parts.push(part);
            }
        }
    }

    Ok(Message {
        role,
        content: parts,
    })
}

fn parse_openai_content_parts(value: &Value) -> Vec<ContentPart> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentPart::Text {
                    text: text.to_string(),
                }]
            }
        }
        Value::Array(items) => {
            let mut out = Vec::<ContentPart>::new();
            for item in items {
                match item {
                    Value::String(text) => {
                        if !text.is_empty() {
                            out.push(ContentPart::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                    Value::Object(obj) => {
                        if let Some(text) = obj.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                out.push(ContentPart::Text {
                                    text: text.to_string(),
                                });
                                continue;
                            }
                        }

                        let ty = obj.get("type").and_then(Value::as_str).unwrap_or_default();
                        match ty {
                            "text" | "input_text" | "output_text" => {
                                if let Some(text) = obj.get("text").and_then(Value::as_str) {
                                    if !text.is_empty() {
                                        out.push(ContentPart::Text {
                                            text: text.to_string(),
                                        });
                                    }
                                }
                            }
                            "image_url" => {
                                if let Some(url) = obj
                                    .get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    out.push(ContentPart::Image {
                                        source: ImageSource::Url {
                                            url: url.to_string(),
                                        },
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn parse_openai_tools(value: &Value) -> ParseResult<Vec<Tool>> {
    let items = value
        .as_array()
        .ok_or_else(|| "tools must be an array".to_string())?;

    let mut out = Vec::<Tool>::new();
    for tool in items {
        let obj = match tool.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let ty = obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if ty != "function" {
            continue;
        }

        let function = obj
            .get("function")
            .and_then(Value::as_object)
            .unwrap_or(obj);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "tool missing function.name".to_string())?;
        let description = function
            .get("description")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let parameters = function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let strict = function.get("strict").and_then(Value::as_bool);

        out.push(Tool {
            name: name.to_string(),
            description,
            parameters,
            strict,
        });
    }
    Ok(out)
}

fn parse_openai_tool_choice(value: &Value) -> ParseResult<Option<ToolChoice>> {
    match value {
        Value::String(choice) => match choice.as_str() {
            "auto" => Ok(Some(ToolChoice::Auto)),
            "none" => Ok(Some(ToolChoice::None)),
            "required" => Ok(Some(ToolChoice::Required)),
            other => Err(format!("unsupported tool_choice: {other}")),
        },
        Value::Object(obj) => {
            let name = obj
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .or_else(|| obj.get("name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "tool_choice missing function.name".to_string())?;
            Ok(Some(ToolChoice::Tool {
                name: name.to_string(),
            }))
        }
        _ => Ok(None),
    }
}

fn parse_openai_tool_call(value: &Value) -> Option<ContentPart> {
    let obj = value.as_object()?;
    let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
    let function = obj.get("function").and_then(Value::as_object)?;
    let name = function.get("name").and_then(Value::as_str)?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let parsed_arguments = serde_json::from_str::<Value>(arguments)
        .unwrap_or_else(|_| Value::String(arguments.into()));

    Some(ContentPart::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: parsed_arguments,
    })
}

fn parse_openai_function_call(obj: &Map<String, Value>) -> Option<ContentPart> {
    let name = obj.get("name").and_then(Value::as_str)?;
    let arguments = obj.get("arguments").and_then(Value::as_str).unwrap_or("{}");
    let parsed_arguments = serde_json::from_str::<Value>(arguments)
        .unwrap_or_else(|_| Value::String(arguments.into()));
    Some(ContentPart::ToolCall {
        id: String::new(),
        name: name.to_string(),
        arguments: parsed_arguments,
    })
}

fn parse_stop_sequences(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(stop) => {
            let stop = stop.trim();
            if stop.is_empty() {
                None
            } else {
                Some(vec![stop.to_string()])
            }
        }
        Value::Array(values) => {
            let mut out = Vec::<String>::new();
            for value in values {
                if let Some(stop) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    out.push(stop.to_string());
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

fn parse_provider_options_from_openai_request(obj: &Map<String, Value>) -> ProviderOptions {
    let mut out = ProviderOptions::default();

    if let Some(reasoning) = obj.get("reasoning").and_then(Value::as_object) {
        if let Some(effort) = reasoning
            .get("effort")
            .and_then(Value::as_str)
            .and_then(parse_reasoning_effort)
        {
            out.reasoning_effort = Some(effort);
        }
    }

    if let Some(parallel) = obj.get("parallel_tool_calls").and_then(Value::as_bool) {
        out.parallel_tool_calls = Some(parallel);
    }

    if let Some(format_value) = obj.get("response_format").and_then(Value::as_object) {
        if let Some(parsed) = parse_json_schema_response_format(format_value) {
            out.response_format = Some(parsed);
        }
    }

    out
}
// end inline: ../../translation/openai_protocol.rs
// inlined from ../../translation/openai_protocol_helpers.rs

fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    match value {
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::XHigh),
        _ => None,
    }
}

fn parse_json_schema_response_format(obj: &Map<String, Value>) -> Option<ResponseFormat> {
    let ty = obj.get("type").and_then(Value::as_str)?;
    if ty != "json_schema" {
        return None;
    }
    serde_json::from_value::<ResponseFormat>(Value::Object(obj.clone())).ok()
}

fn usage_to_chat_usage(usage: &Usage) -> Option<Value> {
    let prompt = usage.input_tokens?;
    let completion = usage.output_tokens?;
    let total = usage
        .total_tokens
        .or_else(|| Some(prompt.saturating_add(completion)))?;
    Some(serde_json::json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    }))
}

fn usage_to_responses_usage(usage: &Usage) -> Option<Value> {
    let mut out = Map::<String, Value>::new();
    if let Some(input_tokens) = usage.input_tokens {
        out.insert(
            "input_tokens".to_string(),
            Value::Number((input_tokens as i64).into()),
        );
    }
    if let Some(output_tokens) = usage.output_tokens {
        out.insert(
            "output_tokens".to_string(),
            Value::Number((output_tokens as i64).into()),
        );
    }
    if let Some(total_tokens) = usage.total_tokens.or_else(|| {
        usage
            .input_tokens
            .zip(usage.output_tokens)
            .map(|(i, o)| i.saturating_add(o))
    }) {
        out.insert(
            "total_tokens".to_string(),
            Value::Number((total_tokens as i64).into()),
        );
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn finish_reason_to_chat_finish_reason(reason: FinishReason) -> Option<&'static str> {
    match reason {
        FinishReason::Stop => Some("stop"),
        FinishReason::Length => Some("length"),
        FinishReason::ToolCalls => Some("tool_calls"),
        FinishReason::ContentFilter => Some("content_filter"),
        FinishReason::Error => Some("error"),
        FinishReason::Unknown => None,
    }
}

fn finish_reason_to_responses_status(reason: FinishReason) -> (&'static str, Option<Value>) {
    match reason {
        FinishReason::Length => (
            "incomplete",
            Some(serde_json::json!({ "reason": "max_output_tokens" })),
        ),
        FinishReason::ContentFilter => (
            "incomplete",
            Some(serde_json::json!({ "reason": "content_filter" })),
        ),
        FinishReason::Error => ("failed", None),
        _ => ("completed", None),
    }
}

fn completion_chunk_bytes(
    id: &str,
    model: &str,
    created: u64,
    text: &str,
    finish_reason: Option<FinishReason>,
) -> Bytes {
    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("text".to_string(), Value::String(text.to_string()));
    choice.insert("logprobs".to_string(), Value::Null);
    if let Some(finish_reason) = finish_reason {
        if let Some(mapped) = finish_reason_to_chat_finish_reason(finish_reason) {
            choice.insert(
                "finish_reason".to_string(),
                Value::String(mapped.to_string()),
            );
        } else {
            choice.insert("finish_reason".to_string(), Value::Null);
        }
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

    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn chat_chunk_bytes(
    id: &str,
    model: &str,
    created: u64,
    delta: Value,
    finish_reason: Option<FinishReason>,
    usage: Option<Value>,
) -> Bytes {
    let mut choice = Map::<String, Value>::new();
    choice.insert("index".to_string(), Value::Number(0.into()));
    choice.insert("delta".to_string(), delta);
    if let Some(finish_reason) = finish_reason {
        if let Some(mapped) = finish_reason_to_chat_finish_reason(finish_reason) {
            choice.insert(
                "finish_reason".to_string(),
                Value::String(mapped.to_string()),
            );
        } else {
            choice.insert("finish_reason".to_string(), Value::Null);
        }
    } else {
        choice.insert("finish_reason".to_string(), Value::Null);
    }

    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion.chunk".to_string()),
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
    if let Some(usage) = usage {
        out.insert("usage".to_string(), usage);
    }

    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn chat_usage_chunk_bytes(id: &str, model: &str, created: u64, usage: Value) -> Bytes {
    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert(
        "object".to_string(),
        Value::String("chat.completion.chunk".to_string()),
    );
    out.insert(
        "created".to_string(),
        Value::Number((created as i64).into()),
    );
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("choices".to_string(), Value::Array(Vec::new()));
    out.insert("usage".to_string(), usage);
    let json = Value::Object(out).to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

fn sse_event_bytes(value: Value) -> Bytes {
    let json = value.to_string();
    Bytes::from(format!("data: {json}\n\n"))
}

pub fn is_files_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/files" || path == "/v1/files/"
}

pub fn files_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

pub fn files_content_id(path_and_query: &str) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    let (file_id, suffix) = rest.split_once('/')?;
    if suffix != "content" {
        return None;
    }
    let file_id = file_id.trim();
    if file_id.is_empty() {
        return None;
    }
    Some(file_id.to_string())
}

pub fn files_upload_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<FileUploadRequest> {
    let mut file: Option<super::multipart::MultipartPart> = None;
    let mut purpose: Option<String> = None;

    let parts = super::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "file" => file = Some(part),
            "purpose" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    purpose = Some(value);
                }
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| "files request missing file".to_string())?;
    let purpose = purpose.ok_or_else(|| "files request missing purpose".to_string())?;
    let filename = file
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "file".to_string());

    Ok(FileUploadRequest {
        filename,
        bytes: file.data.to_vec(),
        purpose,
        media_type: file.content_type.clone(),
    })
}

pub fn file_upload_response_to_openai(
    file_id: &str,
    filename: String,
    purpose: String,
    bytes: usize,
    created_at: u64,
) -> Value {
    serde_json::json!({
        "id": file_id,
        "object": "file",
        "bytes": bytes,
        "created_at": created_at,
        "filename": filename,
        "purpose": purpose,
    })
}

pub fn file_to_openai(file: &crate::file::FileObject) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(file.id.clone()));
    out.insert("object".to_string(), Value::String("file".to_string()));
    out.insert("bytes".to_string(), Value::Number(file.bytes.into()));
    out.insert(
        "created_at".to_string(),
        Value::Number(file.created_at.into()),
    );
    out.insert("filename".to_string(), Value::String(file.filename.clone()));
    out.insert("purpose".to_string(), Value::String(file.purpose.clone()));
    if let Some(status) = file.status.as_deref() {
        out.insert("status".to_string(), Value::String(status.to_string()));
    }
    if let Some(details) = file.status_details.clone() {
        out.insert("status_details".to_string(), details);
    }
    Value::Object(out)
}

pub fn file_list_response_to_openai(files: &[crate::file::FileObject]) -> Value {
    Value::Object(Map::from_iter([
        ("object".to_string(), Value::String("list".to_string())),
        (
            "data".to_string(),
            Value::Array(files.iter().map(file_to_openai).collect()),
        ),
    ]))
}

pub fn file_delete_response_to_openai(response: &crate::file::FileDeleteResponse) -> Value {
    serde_json::json!({
        "id": response.id,
        "object": "file",
        "deleted": response.deleted,
    })
}
// end inline: ../../translation/openai_protocol_helpers.rs
// inlined from ../../translation/files_api.rs
impl TranslationBackend {
    async fn resolve_file_client(&self) -> crate::Result<Arc<dyn FileClient>> {
        self.runtime
            .resolve_file_client(self.provider_name(), self.bindings.file_client.as_ref())
            .await
    }

    pub async fn list_files(&self) -> crate::Result<Vec<crate::file::FileObject>> {
        let client = self.resolve_file_client().await?;
        client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> crate::Result<crate::file::FileObject> {
        let client = self.resolve_file_client().await?;
        client.retrieve_file(file_id).await
    }

    pub async fn delete_file(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileDeleteResponse> {
        let client = self.resolve_file_client().await?;
        client.delete_file(file_id).await
    }

    pub async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileContent> {
        let client = self.resolve_file_client().await?;
        client.download_file_content(file_id).await
    }
}
// end inline: ../../translation/files_api.rs
