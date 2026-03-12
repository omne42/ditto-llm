//! Runtime builder backend factories.
//!
//! This module owns provider-specific adapter instantiation once runtime has
//! already resolved a builder assembly plan.

use std::sync::Arc;

use super::builder_protocol::BuilderAssemblyPlan;
use super::builtin::builtin_runtime_assembly;
use crate::capabilities::audio::{AudioTranscriptionModel, SpeechModel};
use crate::capabilities::embedding::EmbeddingModel;
use crate::capabilities::file::FileClient;
use crate::capabilities::{
    BatchClient, ContextCacheModel, ContextCacheProfile, ImageGenerationModel, ModerationModel,
    RerankModel,
};
use crate::config::Env;
use crate::foundation::error::{DittoError, Result};
use crate::llm_core::model::LanguageModel;

// RUNTIME-BUILDER-BACKEND-OWNER: provider-specific adapter instantiation lives
// here after runtime has already resolved the effective backend/config plan.

pub(super) async fn build_language_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Arc<dyn LanguageModel>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Arc::new(
                    crate::providers::openai::OpenAITextModel::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "openai"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without openai feature".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(feature = "openai-compatible")]
            {
                Ok(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatible::from_config(
                        _config, _env,
                    )
                    .await?,
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
                Ok(Arc::new(
                    crate::providers::anthropic::Anthropic::from_config(_config, _env).await?,
                ))
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
                Ok(Arc::new(
                    crate::providers::google::Google::from_config(_config, _env).await?,
                ))
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
                Ok(Arc::new(
                    crate::providers::cohere::Cohere::from_config(_config, _env).await?,
                ))
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
                Ok(Arc::new(
                    crate::providers::bedrock::Bedrock::from_config(_config, _env).await?,
                ))
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
                Ok(Arc::new(
                    crate::providers::vertex::Vertex::from_config(_config, _env).await?,
                ))
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

pub(super) async fn build_embedding_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn EmbeddingModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "embeddings")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without embeddings support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatibleEmbeddings::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "embeddings")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without embeddings support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        "google" => {
            #[cfg(all(feature = "google", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "google", feature = "embeddings")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without embeddings support for google provider".to_string(),
                ))
            }
        }
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::cohere::CohereEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "cohere", feature = "embeddings")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without embeddings support for cohere provider".to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_moderation_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn ModerationModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "moderations"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIModerations::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "moderations")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without moderations support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "moderations"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_moderations::OpenAICompatibleModerations::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "moderations")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without moderations support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_image_generation_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn ImageGenerationModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "images")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without images support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_images::OpenAICompatibleImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "images")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without images support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        "google" => {
            #[cfg(all(feature = "google", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "google", feature = "images")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without images support for google provider".to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_image_edit_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::ImageEditModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIImageEdits::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "images")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without image edit support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAICompatibleImageEdits::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "images")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without image edit support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_video_generation_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::video::VideoGenerationModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "videos"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIVideos::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "videos")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without videos support for openai provider".to_string(),
                ))
            }
        }
        "google" => {
            #[cfg(all(feature = "google", feature = "videos"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleVideos::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "google", feature = "videos")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without videos support for google provider".to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_realtime_session_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::realtime::RealtimeSessionModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "realtime"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIRealtime::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "realtime")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without realtime support for openai provider".to_string(),
                ))
            }
        }
        "google" => {
            #[cfg(all(feature = "google", feature = "realtime"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleRealtime::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "google", feature = "realtime")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without realtime support for google provider".to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_audio_transcription_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn AudioTranscriptionModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIAudioTranscription::from_config(_config, _env)
                        .await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "audio")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without audio support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_audio::OpenAICompatibleAudioTranscription::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "audio")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without audio support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_speech_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn SpeechModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAISpeech::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "audio")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without audio support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_audio::OpenAICompatibleSpeech::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "audio")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without audio support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_batch_client(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn BatchClient>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "batches"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIBatches::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai", feature = "batches")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without batches support for openai provider".to_string(),
                ))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "openai-compatible", feature = "batches"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_batches::OpenAICompatibleBatches::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "openai-compatible", feature = "batches")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without batches support for openai-compatible providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_rerank_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn RerankModel>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "rerank"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::cohere::CohereRerank::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "cohere", feature = "rerank")))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without rerank support for cohere provider".to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

pub(super) async fn build_file_client(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn FileClient>>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAI::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(feature = "openai"))]
            {
                Ok(None)
            }
        }
        "openai-compatible" => {
            #[cfg(feature = "openai-compatible")]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatible::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(feature = "openai-compatible"))]
            {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

// RUNTIME-CONTEXT-CACHE-BACKEND-OWNER: context cache adapter construction
// stays with the backend owner so public frontdoors do not accumulate provider
// matching or runtime-registry semantics.
#[derive(Clone)]
struct CatalogContextCacheAdapter {
    provider: String,
    model_id: String,
    profile: ContextCacheProfile,
}

impl ContextCacheModel for CatalogContextCacheAdapter {
    fn provider(&self) -> &str {
        self.provider.as_str()
    }

    fn model_id(&self) -> &str {
        self.model_id.as_str()
    }

    fn context_cache_profile(&self) -> &ContextCacheProfile {
        &self.profile
    }
}

pub(super) async fn build_context_cache_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Option<Arc<dyn ContextCacheModel>>> {
    let model_id = plan
        .config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DittoError::InvalidResponse(format!(
                "context cache model is not set for provider {}",
                plan.behavior_provider
            ))
        })?;
    let profile = builtin_runtime_assembly()
        .registry()
        .resolve_context_cache_profile(plan.behavior_provider, &plan.config, model_id)
        .ok_or_else(|| {
            DittoError::InvalidResponse(format!(
                "provider {} resolved context.cache but runtime_registry produced an empty context cache profile",
                plan.behavior_provider
            ))
        })?;

    Ok(Some(Arc::new(CatalogContextCacheAdapter {
        provider: plan.behavior_provider.to_string(),
        model_id: model_id.to_string(),
        profile,
    })))
}
