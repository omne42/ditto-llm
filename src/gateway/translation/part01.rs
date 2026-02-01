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
use crate::file::{FileClient, FileUploadRequest};
use crate::image::ImageGenerationModel;
use crate::model::{LanguageModel, StreamResult};
use crate::moderation::ModerationModel;
use crate::object::{LanguageModelObjectExt, ObjectOptions, ObjectOutput};
use crate::rerank::RerankModel;
use crate::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, ContentPart, FinishReason, GenerateRequest, GenerateResponse,
    ImageGenerationRequest, ImageGenerationResponse, ImageSource, JsonSchemaFormat, Message,
    ModerationInput, ModerationRequest, ModerationResponse, ProviderOptions, ReasoningEffort,
    RerankRequest, RerankResponse, ResponseFormat, Role, SpeechRequest, SpeechResponse,
    SpeechResponseFormat, Tool, ToolChoice, TranscriptionResponseFormat, Usage,
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
    pub file_client: Option<Arc<dyn FileClient>>,
    pub provider: String,
    pub model_map: BTreeMap<String, String>,
    env: Env,
    provider_config: ProviderConfig,
    embedding_cache: Arc<Mutex<HashMap<String, Arc<dyn EmbeddingModel>>>>,
    moderation_cache: Arc<Mutex<Option<Arc<dyn ModerationModel>>>>,
    image_generation_cache: Arc<Mutex<Option<Arc<dyn ImageGenerationModel>>>>,
    audio_transcription_cache: Arc<Mutex<HashMap<String, Arc<dyn AudioTranscriptionModel>>>>,
    speech_cache: Arc<Mutex<HashMap<String, Arc<dyn SpeechModel>>>>,
    rerank_cache: Arc<Mutex<HashMap<String, Arc<dyn RerankModel>>>>,
    batch_cache: Arc<Mutex<Option<Arc<dyn BatchClient>>>>,
    file_cache: Arc<Mutex<Option<Arc<dyn FileClient>>>>,
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
            file_client: None,
            provider: provider.into(),
            model_map: BTreeMap::new(),
            env: Env::default(),
            provider_config: ProviderConfig::default(),
            embedding_cache: Arc::new(Mutex::new(HashMap::new())),
            moderation_cache: Arc::new(Mutex::new(None)),
            image_generation_cache: Arc::new(Mutex::new(None)),
            audio_transcription_cache: Arc::new(Mutex::new(HashMap::new())),
            speech_cache: Arc::new(Mutex::new(HashMap::new())),
            rerank_cache: Arc::new(Mutex::new(HashMap::new())),
            batch_cache: Arc::new(Mutex::new(None)),
            file_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_provider_config(mut self, provider_config: ProviderConfig) -> Self {
        self.provider_config = provider_config;
        self
    }

    pub fn with_env(mut self, env: Env) -> Self {
        self.env = env;
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

    pub fn with_file_client(mut self, file_client: Arc<dyn FileClient>) -> Self {
        self.file_client = Some(file_client);
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

    pub async fn upload_file(&self, request: FileUploadRequest) -> crate::Result<String> {
        if let Some(client) = self.file_client.as_ref() {
            return client.upload_file_with_purpose(request).await;
        }

        let cached = self.file_cache.lock().await.clone();
        if let Some(client) = cached {
            return client.upload_file_with_purpose(request).await;
        }

        let client = build_file_client(self.provider.as_str(), &self.provider_config, &self.env)
            .await?
            .ok_or_else(|| {
                DittoError::InvalidResponse(format!(
                    "provider backend does not support files: {}",
                    self.provider
                ))
            })?;

        {
            let mut cache = self.file_cache.lock().await;
            *cache = Some(client.clone());
        }

        client.upload_file_with_purpose(request).await
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

        let model_impl = build_embedding_model(self.provider.as_str(), &cfg, &self.env)
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

        let model_impl =
            build_moderation_model(self.provider.as_str(), &self.provider_config, &self.env)
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

        let model_impl =
            build_image_generation_model(self.provider.as_str(), &self.provider_config, &self.env)
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

        let model_impl = build_audio_transcription_model(self.provider.as_str(), &cfg, &self.env)
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

        let model_impl = build_speech_model(self.provider.as_str(), &cfg, &self.env)
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

        let model_impl = build_rerank_model(self.provider.as_str(), &cfg, &self.env)
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

        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &self.env)
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

        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &self.env)
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

        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &self.env)
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

        let client = build_batch_client(self.provider.as_str(), &self.provider_config, &self.env)
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "cohere" => {
            #[cfg(feature = "cohere")]
            {
                Ok(Arc::new(crate::Cohere::from_config(config, env).await?))
            }
            #[cfg(not(feature = "cohere"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without cohere feature".to_string(),
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
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
