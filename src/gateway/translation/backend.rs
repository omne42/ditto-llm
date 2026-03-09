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
