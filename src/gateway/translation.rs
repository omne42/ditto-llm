use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use axum::http::StatusCode;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::audio::{AudioTranscriptionModel, SpeechModel};
use crate::batch::BatchClient;
use crate::embedding::EmbeddingModel;
use crate::image::ImageGenerationModel;
use crate::model::{LanguageModel, StreamResult};
use crate::moderation::ModerationModel;
use crate::rerank::RerankModel;
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, ContentPart, FinishReason, GenerateRequest, GenerateResponse,
    ImageGenerationRequest, ImageGenerationResponse, ImageSource, Message, ModerationInput,
    ModerationRequest, ModerationResponse, ProviderOptions, ReasoningEffort, RerankRequest,
    RerankResponse, ResponseFormat, Role, SpeechRequest, SpeechResponse, SpeechResponseFormat,
    Tool, ToolChoice, TranscriptionResponseFormat, Usage,
};
use crate::{DittoError, Env, ProviderConfig};

type ParseResult<T> = std::result::Result<T, String>;
type IoResult<T> = std::result::Result<T, std::io::Error>;

#[derive(Clone)]
pub struct TranslationBackend {
    pub model: Arc<dyn LanguageModel>,
    pub embedding_model: Option<Arc<dyn EmbeddingModel>>,
    pub image_generation_model: Option<Arc<dyn ImageGenerationModel>>,
    pub moderation_model: Option<Arc<dyn ModerationModel>>,
    pub audio_transcription_model: Option<Arc<dyn AudioTranscriptionModel>>,
    pub speech_model: Option<Arc<dyn SpeechModel>>,
    pub rerank_model: Option<Arc<dyn RerankModel>>,
    pub batch_client: Option<Arc<dyn BatchClient>>,
    pub provider: String,
    pub model_map: BTreeMap<String, String>,
    provider_config: ProviderConfig,
    embedding_cache: Arc<Mutex<HashMap<String, Arc<dyn EmbeddingModel>>>>,
    moderation_cache: Arc<Mutex<Option<Arc<dyn ModerationModel>>>>,
    image_generation_cache: Arc<Mutex<Option<Arc<dyn ImageGenerationModel>>>>,
    audio_transcription_cache: Arc<Mutex<HashMap<String, Arc<dyn AudioTranscriptionModel>>>>,
    speech_cache: Arc<Mutex<HashMap<String, Arc<dyn SpeechModel>>>>,
    rerank_cache: Arc<Mutex<HashMap<String, Arc<dyn RerankModel>>>>,
    batch_cache: Arc<Mutex<Option<Arc<dyn BatchClient>>>>,
}

impl TranslationBackend {
    pub fn new(provider: impl Into<String>, model: Arc<dyn LanguageModel>) -> Self {
        Self {
            model,
            embedding_model: None,
            image_generation_model: None,
            moderation_model: None,
            audio_transcription_model: None,
            speech_model: None,
            rerank_model: None,
            batch_client: None,
            provider: provider.into(),
            model_map: BTreeMap::new(),
            provider_config: ProviderConfig::default(),
            embedding_cache: Arc::new(Mutex::new(HashMap::new())),
            moderation_cache: Arc::new(Mutex::new(None)),
            image_generation_cache: Arc::new(Mutex::new(None)),
            audio_transcription_cache: Arc::new(Mutex::new(HashMap::new())),
            speech_cache: Arc::new(Mutex::new(HashMap::new())),
            rerank_cache: Arc::new(Mutex::new(HashMap::new())),
            batch_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_provider_config(mut self, provider_config: ProviderConfig) -> Self {
        self.provider_config = provider_config;
        self
    }

    pub fn with_model_map(mut self, model_map: BTreeMap<String, String>) -> Self {
        self.model_map = model_map;
        self
    }

    pub fn with_embedding_model(mut self, embedding_model: Arc<dyn EmbeddingModel>) -> Self {
        self.embedding_model = Some(embedding_model);
        self
    }

    pub fn with_image_generation_model(
        mut self,
        image_generation_model: Arc<dyn ImageGenerationModel>,
    ) -> Self {
        self.image_generation_model = Some(image_generation_model);
        self
    }

    pub fn with_moderation_model(mut self, moderation_model: Arc<dyn ModerationModel>) -> Self {
        self.moderation_model = Some(moderation_model);
        self
    }

    pub fn with_audio_transcription_model(
        mut self,
        audio_transcription_model: Arc<dyn AudioTranscriptionModel>,
    ) -> Self {
        self.audio_transcription_model = Some(audio_transcription_model);
        self
    }

    pub fn with_speech_model(mut self, speech_model: Arc<dyn SpeechModel>) -> Self {
        self.speech_model = Some(speech_model);
        self
    }

    pub fn with_rerank_model(mut self, rerank_model: Arc<dyn RerankModel>) -> Self {
        self.rerank_model = Some(rerank_model);
        self
    }

    pub fn with_batch_client(mut self, batch_client: Arc<dyn BatchClient>) -> Self {
        self.batch_client = Some(batch_client);
        self
    }

    pub fn map_model(&self, requested: &str) -> String {
        if let Some(mapped) = self.model_map.get(requested) {
            return mapped.clone();
        }

        let requested = requested.trim();
        if requested.is_empty() {
            return String::new();
        }

        let prefix = format!("{}/", self.provider.trim());
        if prefix != "/" && requested.starts_with(&prefix) {
            return requested.trim_start_matches(&prefix).to_string();
        }

        requested.to_string()
    }

    pub async fn embed(&self, model: &str, texts: Vec<String>) -> crate::Result<Vec<Vec<f32>>> {
        if let Some(model_impl) = self.embedding_model.as_ref() {
            return model_impl.embed(texts).await;
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "embedding model is missing".to_string(),
            ));
        }

        if let Some(model_impl) = self.embedding_cache.lock().await.get(model).cloned() {
            return model_impl.embed(texts).await;
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl = build_embedding_model(self.provider.as_str(), &cfg, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support embeddings: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.embedding_cache.lock().await;
            cache.insert(model.to_string(), model_impl.clone());
        }

        model_impl.embed(texts).await
    }

    pub async fn moderate(&self, request: ModerationRequest) -> crate::Result<ModerationResponse> {
        if let Some(model_impl) = self.moderation_model.as_ref() {
            return model_impl.moderate(request).await;
        }

        let cached = self.moderation_cache.lock().await.clone();
        if let Some(model_impl) = cached {
            return model_impl.moderate(request).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl =
            build_moderation_model(self.provider.as_str(), &self.provider_config, &env)
                .await?
                .ok_or_else(|| {
                    DittoError::InvalidResponse(format!(
                        "provider backend does not support moderations: {}",
                        self.provider
                    ))
                })?;

        {
            let mut cache = self.moderation_cache.lock().await;
            *cache = Some(model_impl.clone());
        }

        model_impl.moderate(request).await
    }

    pub async fn generate_image(
        &self,
        request: ImageGenerationRequest,
    ) -> crate::Result<ImageGenerationResponse> {
        if let Some(model_impl) = self.image_generation_model.as_ref() {
            return model_impl.generate(request).await;
        }

        let cached = self.image_generation_cache.lock().await.clone();
        if let Some(model_impl) = cached {
            return model_impl.generate(request).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl =
            build_image_generation_model(self.provider.as_str(), &self.provider_config, &env)
                .await?
                .ok_or_else(|| {
                    DittoError::InvalidResponse(format!(
                        "provider backend does not support images: {}",
                        self.provider
                    ))
                })?;

        {
            let mut cache = self.image_generation_cache.lock().await;
            *cache = Some(model_impl.clone());
        }

        model_impl.generate(request).await
    }

    pub async fn transcribe_audio(
        &self,
        model: &str,
        mut request: AudioTranscriptionRequest,
    ) -> crate::Result<AudioTranscriptionResponse> {
        if let Some(model_impl) = self.audio_transcription_model.as_ref() {
            if request
                .model
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                request.model = Some(model.trim().to_string());
            }
            return model_impl.transcribe(request).await;
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "audio transcription model is missing".to_string(),
            ));
        }

        if let Some(model_impl) = self
            .audio_transcription_cache
            .lock()
            .await
            .get(model)
            .cloned()
        {
            request.model = Some(model.to_string());
            return model_impl.transcribe(request).await;
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl = build_audio_transcription_model(self.provider.as_str(), &cfg, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support audio transcriptions: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.audio_transcription_cache.lock().await;
            cache.insert(model.to_string(), model_impl.clone());
        }

        request.model = Some(model.to_string());
        model_impl.transcribe(request).await
    }

    pub async fn speak_audio(
        &self,
        model: &str,
        mut request: SpeechRequest,
    ) -> crate::Result<SpeechResponse> {
        if let Some(model_impl) = self.speech_model.as_ref() {
            if request
                .model
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                request.model = Some(model.trim().to_string());
            }
            return model_impl.speak(request).await;
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "speech model is missing".to_string(),
            ));
        }

        if let Some(model_impl) = self.speech_cache.lock().await.get(model).cloned() {
            request.model = Some(model.to_string());
            return model_impl.speak(request).await;
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl = build_speech_model(self.provider.as_str(), &cfg, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support audio speech: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.speech_cache.lock().await;
            cache.insert(model.to_string(), model_impl.clone());
        }

        request.model = Some(model.to_string());
        model_impl.speak(request).await
    }

    pub async fn rerank(
        &self,
        model: &str,
        mut request: RerankRequest,
    ) -> crate::Result<RerankResponse> {
        if let Some(model_impl) = self.rerank_model.as_ref() {
            if request
                .model
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                request.model = Some(model.trim().to_string());
            }
            return model_impl.rerank(request).await;
        }

        let model = model.trim();
        if model.is_empty() {
            return Err(DittoError::InvalidResponse(
                "rerank model is missing".to_string(),
            ));
        }

        if let Some(model_impl) = self.rerank_cache.lock().await.get(model).cloned() {
            request.model = Some(model.to_string());
            return model_impl.rerank(request).await;
        }

        let mut cfg = self.provider_config.clone();
        cfg.default_model = Some(model.to_string());

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let model_impl = build_rerank_model(self.provider.as_str(), &cfg, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support rerank: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.rerank_cache.lock().await;
            cache.insert(model.to_string(), model_impl.clone());
        }

        request.model = Some(model.to_string());
        model_impl.rerank(request).await
    }

    pub async fn create_batch(&self, request: BatchCreateRequest) -> crate::Result<BatchResponse> {
        if let Some(client) = self.batch_client.as_ref() {
            return client.create(request).await;
        }

        let cached = self.batch_cache.lock().await.clone();
        if let Some(client) = cached {
            return client.create(request).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support batches: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.batch_cache.lock().await;
            *cache = Some(client.clone());
        }

        client.create(request).await
    }

    pub async fn retrieve_batch(&self, batch_id: &str) -> crate::Result<BatchResponse> {
        if let Some(client) = self.batch_client.as_ref() {
            return client.retrieve(batch_id).await;
        }

        let cached = self.batch_cache.lock().await.clone();
        if let Some(client) = cached {
            return client.retrieve(batch_id).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support batches: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.batch_cache.lock().await;
            *cache = Some(client.clone());
        }

        client.retrieve(batch_id).await
    }

    pub async fn cancel_batch(&self, batch_id: &str) -> crate::Result<BatchResponse> {
        if let Some(client) = self.batch_client.as_ref() {
            return client.cancel(batch_id).await;
        }

        let cached = self.batch_cache.lock().await.clone();
        if let Some(client) = cached {
            return client.cancel(batch_id).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support batches: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.batch_cache.lock().await;
            *cache = Some(client.clone());
        }

        client.cancel(batch_id).await
    }

    pub async fn list_batches(
        &self,
        limit: Option<u32>,
        after: Option<String>,
    ) -> crate::Result<BatchListResponse> {
        if let Some(client) = self.batch_client.as_ref() {
            return client.list(limit, after).await;
        }

        let cached = self.batch_cache.lock().await.clone();
        if let Some(client) = cached {
            return client.list(limit, after).await;
        }

        let env = Env {
            dotenv: BTreeMap::new(),
        };
        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support batches: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.batch_cache.lock().await;
            *cache = Some(client.clone());
        }

        client.list(limit, after).await
    }
}

pub async fn build_language_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Arc<dyn LanguageModel>> {
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Arc::new(crate::OpenAI::from_config(config, env).await?))
            }
            #[cfg(not(feature = "openai"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without openai feature".to_string(),
                ))
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(feature = "openai-compatible")]
            {
                Ok(Arc::new(
                    crate::OpenAICompatible::from_config(config, env).await?,
                ))
            }
            #[cfg(not(feature = "openai-compatible"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without openai-compatible feature".to_string(),
                ))
            }
        }
        "anthropic" => {
            #[cfg(feature = "anthropic")]
            {
                Ok(Arc::new(crate::Anthropic::from_config(config, env).await?))
            }
            #[cfg(not(feature = "anthropic"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without anthropic feature".to_string(),
                ))
            }
        }
        "google" => {
            #[cfg(feature = "google")]
            {
                Ok(Arc::new(crate::Google::from_config(config, env).await?))
            }
            #[cfg(not(feature = "google"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without google feature".to_string(),
                ))
            }
        }
        "bedrock" => {
            #[cfg(feature = "bedrock")]
            {
                Ok(Arc::new(crate::Bedrock::from_config(config, env).await?))
            }
            #[cfg(not(feature = "bedrock"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without bedrock feature".to_string(),
                ))
            }
        }
        "vertex" => {
            #[cfg(feature = "vertex")]
            {
                Ok(Arc::new(crate::Vertex::from_config(config, env).await?))
            }
            #[cfg(not(feature = "vertex"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without vertex feature".to_string(),
                ))
            }
        }
        other => Err(DittoError::InvalidResponse(format!(
            "unsupported provider backend: {other}"
        ))),
    }
}

pub async fn build_embedding_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn EmbeddingModel>>> {
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIEmbeddings::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "embeddings")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleEmbeddings::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "embeddings")))]
            {
                Ok(None)
            }
        }
        "google" => {
            #[cfg(all(feature = "google", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::GoogleEmbeddings::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "google", feature = "embeddings")))]
            {
                Ok(None)
            }
        }
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::CohereEmbeddings::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "cohere", feature = "embeddings")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_moderation_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn ModerationModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "moderations"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIModerations::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "moderations")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "moderations"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleModerations::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "moderations")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_image_generation_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn ImageGenerationModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIImages::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "images")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleImages::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "images")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_audio_transcription_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn AudioTranscriptionModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIAudioTranscription::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "audio")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleAudioTranscription::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "audio")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_speech_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn SpeechModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAISpeech::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "audio")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleSpeech::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "audio")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_batch_client(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn BatchClient>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "batches"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIBatches::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "batches")))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "batches"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatibleBatches::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "batches")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

pub async fn build_rerank_model(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn RerankModel>>> {
    let _ = (config, env);
    let provider = provider.trim();
    match provider {
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "rerank"))]
            {
                Ok(Some(Arc::new(
                    crate::CohereRerank::from_config(config, env).await?,
                )))
            }
            #[cfg(not(all(feature = "cohere", feature = "rerank")))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

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

pub fn is_audio_transcriptions_path(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    path == "/v1/audio/transcriptions" || path == "/v1/audio/transcriptions/"
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

        let provider = backend.provider.trim();
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

        let default_model = backend.model.model_id().trim();
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

#[derive(Debug, Clone)]
struct MultipartPart {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Bytes,
}

fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    if start >= haystack.len() {
        return None;
    }
    let first = needle[0];
    let mut pos = start;
    while pos + needle.len() <= haystack.len() {
        let rel = haystack[pos..].iter().position(|&b| b == first)?;
        pos += rel;
        if pos + needle.len() > haystack.len() {
            return None;
        }
        if &haystack[pos..pos + needle.len()] == needle {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn multipart_boundary(content_type: &str) -> ParseResult<String> {
    for part in content_type.split(';').map(str::trim) {
        if part.len() < "boundary=".len() {
            continue;
        }
        if !part[..].to_ascii_lowercase().starts_with("boundary=") {
            continue;
        }

        let value = part["boundary=".len()..].trim();
        if value.is_empty() {
            continue;
        }

        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);

        if unquoted.trim().is_empty() {
            continue;
        }

        return Ok(unquoted.to_string());
    }

    Err("multipart boundary is missing".to_string())
}

fn parse_multipart_form(content_type: &str, body: &Bytes) -> ParseResult<Vec<MultipartPart>> {
    let boundary = multipart_boundary(content_type)?;
    let boundary_marker = format!("--{boundary}");
    let boundary_bytes = boundary_marker.as_bytes();
    let delimiter = format!("\r\n{boundary_marker}");
    let delimiter_bytes = delimiter.as_bytes();

    let bytes = body.as_ref();
    let Some(mut cursor) = find_subslice(bytes, boundary_bytes, 0) else {
        return Err("multipart body missing boundary marker".to_string());
    };
    cursor += boundary_bytes.len();

    let mut parts = Vec::<MultipartPart>::new();
    loop {
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }

        let (headers_end, header_sep_len) =
            if let Some(idx) = find_subslice(bytes, b"\r\n\r\n", cursor) {
                (idx, 4)
            } else if let Some(idx) = find_subslice(bytes, b"\n\n", cursor) {
                (idx, 2)
            } else {
                return Err("multipart part missing header separator".to_string());
            };

        let headers_raw = String::from_utf8_lossy(&bytes[cursor..headers_end]);
        let mut name: Option<String> = None;
        let mut filename: Option<String> = None;
        let mut content_type: Option<String> = None;

        for line in headers_raw.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("content-disposition") {
                for item in value.split(';').map(str::trim) {
                    if let Some(value) = item.strip_prefix("name=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        name = Some(value.to_string());
                    } else if let Some(value) = item.strip_prefix("filename=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        filename = Some(value.to_string());
                    }
                }
            } else if key.eq_ignore_ascii_case("content-type") && !value.is_empty() {
                content_type = Some(value.to_string());
            }
        }

        let name =
            name.ok_or_else(|| "multipart part missing content-disposition name".to_string())?;
        let data_start = headers_end + header_sep_len;

        let Some(delim_pos) = find_subslice(bytes, delimiter_bytes, data_start) else {
            return Err("multipart part missing trailing boundary".to_string());
        };
        let data_end = delim_pos;

        let data = body.slice(data_start..data_end);
        parts.push(MultipartPart {
            name,
            filename,
            content_type,
            data,
        });

        cursor = delim_pos + delimiter_bytes.len();
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }
    }

    Ok(parts)
}

pub fn multipart_extract_text_field(
    content_type: &str,
    body: &Bytes,
    field_name: &str,
) -> ParseResult<Option<String>> {
    let parts = parse_multipart_form(content_type, body)?;
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
    let mut file: Option<MultipartPart> = None;
    let mut model: Option<String> = None;
    let mut language: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<TranscriptionResponseFormat> = None;
    let mut temperature: Option<f32> = None;

    let parts = parse_multipart_form(content_type, body)?;
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
            serde_json::to_value(provider_options)
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
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        out.max_tokens = Some(max_tokens.min(u64::from(u32::MAX)) as u32);
    }
    if let Some(stop) = obj.get("stop") {
        out.stop_sequences = parse_stop_sequences(stop);
    }

    let provider_options = parse_provider_options_from_openai_request(obj);
    if provider_options != ProviderOptions::default() {
        out.provider_options = Some(
            serde_json::to_value(provider_options)
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
    if let Some(existing) = out
        .provider_options
        .as_ref()
        .and_then(|value| ProviderOptions::from_value(value).ok())
    {
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
            serde_json::to_value(provider_options)
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

pub fn generate_response_to_chat_completions(
    response: &GenerateResponse,
    id: &str,
    model: &str,
    created: u64,
) -> Value {
    let mut content = String::new();
    let mut tool_calls = Vec::<Value>::new();
    for (idx, part) in response.content.iter().enumerate() {
        match part {
            ContentPart::Text { text } => content.push_str(text),
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
                                crate::types::StreamChunk::ReasoningDelta { .. } => {}
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
                            if let Some(usage) = state.usage.as_ref().and_then(usage_to_chat_usage)
                            {
                                buffer.push_back(Ok(chat_usage_chunk_bytes(
                                    &state.response_id,
                                    &model,
                                    created,
                                    usage,
                                )));
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
                            crate::types::StreamChunk::ReasoningDelta { .. } => {}
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
        .or(Some(prompt.saturating_add(completion)))?;
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
