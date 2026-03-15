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
use crate::error::{DittoError, Result};
use crate::llm_core::model::LanguageModel;

// RUNTIME-BUILDER-BACKEND-OWNER: provider-specific adapter instantiation lives
// here after runtime has already resolved the effective backend/config plan.

#[allow(dead_code)]
fn provider_feature_missing(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.provider_feature_missing",
        "provider" => provider
    )
}

#[allow(dead_code)]
fn capability_feature_missing(provider: &str, capability: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.capability_feature_missing",
        "provider" => provider,
        "capability" => capability
    )
}

#[allow(dead_code)]
fn unsupported_provider_backend(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.unsupported_provider_backend",
        "provider" => provider
    )
}

#[allow(dead_code)]
fn context_cache_model_missing(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.context_cache_model_missing",
        "provider" => provider
    )
}

#[allow(dead_code)]
fn context_cache_profile_missing(provider: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.builder.context_cache_profile_missing",
        "provider" => provider
    )
}

pub(super) async fn build_language_model(
    plan: &BuilderAssemblyPlan,
    _env: &Env,
) -> Result<Arc<dyn LanguageModel>> {
    let provider = plan.provider;
    let _config = &plan.config;
    match provider {
        "openai" => {
            #[cfg(feature = "provider-openai")]
            {
                Ok(Arc::new(
                    crate::providers::openai::OpenAITextModel::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-openai"))]
            {
                Err(provider_feature_missing("openai"))
            }
        }
        "openai-compatible" => {
            #[cfg(feature = "provider-openai-compatible")]
            {
                Ok(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatible::from_config(
                        _config, _env,
                    )
                    .await?,
                ))
            }
            #[cfg(not(feature = "provider-openai-compatible"))]
            {
                Err(provider_feature_missing("openai-compatible"))
            }
        }
        "anthropic" => {
            #[cfg(feature = "provider-anthropic")]
            {
                Ok(Arc::new(
                    crate::providers::anthropic::Anthropic::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-anthropic"))]
            {
                Err(provider_feature_missing("anthropic"))
            }
        }
        "google" => {
            #[cfg(feature = "provider-google")]
            {
                Ok(Arc::new(
                    crate::providers::google::Google::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-google"))]
            {
                Err(provider_feature_missing("google"))
            }
        }
        "cohere" => {
            #[cfg(feature = "provider-cohere")]
            {
                Ok(Arc::new(
                    crate::providers::cohere::Cohere::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-cohere"))]
            {
                Err(provider_feature_missing("cohere"))
            }
        }
        "bedrock" => {
            #[cfg(feature = "provider-bedrock")]
            {
                Ok(Arc::new(
                    crate::providers::bedrock::Bedrock::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-bedrock"))]
            {
                Err(provider_feature_missing("bedrock"))
            }
        }
        "vertex" => {
            #[cfg(feature = "provider-vertex")]
            {
                Ok(Arc::new(
                    crate::providers::vertex::Vertex::from_config(_config, _env).await?,
                ))
            }
            #[cfg(not(feature = "provider-vertex"))]
            {
                Err(provider_feature_missing("vertex"))
            }
        }
        other => Err(unsupported_provider_backend(other)),
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
            #[cfg(all(feature = "provider-openai", feature = "cap-embedding"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai", feature = "cap-embedding")))]
            {
                Err(capability_feature_missing("openai", "embedding"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "provider-openai-compatible", feature = "cap-embedding"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatibleEmbeddings::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai-compatible", feature = "cap-embedding")))]
            {
                Err(capability_feature_missing("openai-compatible", "embedding"))
            }
        }
        "google" => {
            #[cfg(all(feature = "provider-google", feature = "cap-embedding"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-google", feature = "cap-embedding")))]
            {
                Err(capability_feature_missing("google", "embedding"))
            }
        }
        "cohere" => {
            #[cfg(all(feature = "provider-cohere", feature = "cap-embedding"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::cohere::CohereEmbeddings::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-cohere", feature = "cap-embedding")))]
            {
                Err(capability_feature_missing("cohere", "embedding"))
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
            #[cfg(all(feature = "provider-openai", feature = "cap-moderation"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIModerations::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai", feature = "cap-moderation")))]
            {
                Err(capability_feature_missing("openai", "moderation"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "provider-openai-compatible", feature = "cap-moderation"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_moderations::OpenAICompatibleModerations::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai-compatible", feature = "cap-moderation")))]
            {
                Err(capability_feature_missing(
                    "openai-compatible",
                    "moderation",
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
            #[cfg(all(
                feature = "provider-openai",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            )))]
            {
                Err(capability_feature_missing("openai", "image"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_images::OpenAICompatibleImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            )))]
            {
                Err(capability_feature_missing("openai-compatible", "image"))
            }
        }
        "google" => {
            #[cfg(all(
                feature = "provider-google",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleImages::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-google",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            )))]
            {
                Err(capability_feature_missing("google", "image"))
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
            #[cfg(all(
                feature = "provider-openai",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIImageEdits::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            )))]
            {
                Err(capability_feature_missing("openai", "image_edit"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAICompatibleImageEdits::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-image-generation", feature = "cap-image-edit")
            )))]
            {
                Err(capability_feature_missing(
                    "openai-compatible",
                    "image_edit",
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
            #[cfg(all(feature = "provider-openai", feature = "cap-video-generation"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIVideos::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai", feature = "cap-video-generation")))]
            {
                Err(capability_feature_missing("openai", "video_generation"))
            }
        }
        "google" => {
            #[cfg(all(feature = "provider-google", feature = "cap-video-generation"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleVideos::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-google", feature = "cap-video-generation")))]
            {
                Err(capability_feature_missing("google", "video_generation"))
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
            #[cfg(all(feature = "provider-openai", feature = "cap-realtime"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIRealtime::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai", feature = "cap-realtime")))]
            {
                Err(capability_feature_missing("openai", "realtime"))
            }
        }
        "google" => {
            #[cfg(all(feature = "provider-google", feature = "cap-realtime"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::google::GoogleRealtime::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-google", feature = "cap-realtime")))]
            {
                Err(capability_feature_missing("google", "realtime"))
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
            #[cfg(all(
                feature = "provider-openai",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIAudioTranscription::from_config(_config, _env)
                        .await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            )))]
            {
                Err(capability_feature_missing("openai", "audio"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_audio::OpenAICompatibleAudioTranscription::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            )))]
            {
                Err(capability_feature_missing("openai-compatible", "audio"))
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
            #[cfg(all(
                feature = "provider-openai",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAISpeech::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            )))]
            {
                Err(capability_feature_missing("openai", "audio"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            ))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_audio::OpenAICompatibleSpeech::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(all(
                feature = "provider-openai-compatible",
                any(feature = "cap-audio-transcription", feature = "cap-audio-speech")
            )))]
            {
                Err(capability_feature_missing("openai-compatible", "audio"))
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
            #[cfg(all(feature = "provider-openai", feature = "cap-batch"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAIBatches::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai", feature = "cap-batch")))]
            {
                Err(capability_feature_missing("openai", "batch"))
            }
        }
        "openai-compatible" => {
            #[cfg(all(feature = "provider-openai-compatible", feature = "cap-batch"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible_batches::OpenAICompatibleBatches::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-openai-compatible", feature = "cap-batch")))]
            {
                Err(capability_feature_missing("openai-compatible", "batch"))
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
            #[cfg(all(feature = "provider-cohere", feature = "cap-rerank"))]
            {
                Ok(Some(Arc::new(
                    crate::providers::cohere::CohereRerank::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(all(feature = "provider-cohere", feature = "cap-rerank")))]
            {
                Err(capability_feature_missing("cohere", "rerank"))
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
            #[cfg(feature = "provider-openai")]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai::OpenAI::from_config(_config, _env).await?,
                )))
            }
            #[cfg(not(feature = "provider-openai"))]
            {
                Ok(None)
            }
        }
        "openai-compatible" => {
            #[cfg(feature = "provider-openai-compatible")]
            {
                Ok(Some(Arc::new(
                    crate::providers::openai_compatible::OpenAICompatible::from_config(
                        _config, _env,
                    )
                    .await?,
                )))
            }
            #[cfg(not(feature = "provider-openai-compatible"))]
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
        .ok_or_else(|| context_cache_model_missing(plan.behavior_provider))?;
    let profile = builtin_runtime_assembly()
        .registry()
        .resolve_context_cache_profile(plan.behavior_provider, &plan.config, model_id)
        .ok_or_else(|| context_cache_profile_missing(plan.behavior_provider))?;

    Ok(Some(Arc::new(CatalogContextCacheAdapter {
        provider: plan.behavior_provider.to_string(),
        model_id: model_id.to_string(),
        profile,
    })))
}
