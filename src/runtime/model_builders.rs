//! Runtime provider model builders.
//!
//! These builders are the runtime assembly layer that turns resolved provider/config input
//! into concrete capability adapters. `gateway` consumes them, but does not own them.

use std::sync::Arc;

use crate::audio::{AudioTranscriptionModel, SpeechModel};
use crate::batch::BatchClient;
use crate::embedding::EmbeddingModel;
use crate::file::FileClient;
use crate::image::ImageGenerationModel;
use crate::model::LanguageModel;
use crate::moderation::ModerationModel;
use crate::rerank::RerankModel;
use crate::{DittoError, Env, ProviderConfig};

fn configured_default_model(config: &ProviderConfig) -> Option<&str> {
    config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

fn canonical_builder_provider_from_plugin(
    plugin: &crate::ProviderPluginDescriptor,
) -> Option<&'static str> {
    match plugin.id {
        "openai" => Some("openai"),
        "anthropic" => Some("anthropic"),
        "google" => Some("google"),
        "cohere" => Some("cohere"),
        "bedrock" => Some("bedrock"),
        "vertex" => Some("vertex"),
        _ => match plugin.class {
            crate::ProviderClass::GenericOpenAi | crate::ProviderClass::OpenAiCompatible => {
                Some("openai-compatible")
            }
            _ => None,
        },
    }
}

fn canonical_builder_provider_from_hint(provider: &str) -> Option<&'static str> {
    match provider.trim() {
        "openai" => Some("openai"),
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => Some("openai-compatible"),
        "anthropic" => Some("anthropic"),
        "google" => Some("google"),
        "cohere" => Some("cohere"),
        "bedrock" => Some("bedrock"),
        "vertex" => Some("vertex"),
        _ => None,
    }
}

fn builder_plugin_from_upstream_api(
    config: &ProviderConfig,
) -> Option<&'static crate::ProviderPluginDescriptor> {
    let registry = crate::builtin_registry();
    match config.upstream_api {
        Some(crate::ProviderApi::GeminiGenerateContent) => registry.plugin("google"),
        Some(crate::ProviderApi::AnthropicMessages) => registry.plugin("anthropic"),
        Some(crate::ProviderApi::OpenaiChatCompletions)
        | Some(crate::ProviderApi::OpenaiResponses)
        | None => registry
            .plugin("openai-compatible")
            .or_else(|| registry.plugin("openai")),
    }
}

fn resolve_builder_plugin(
    provider: &str,
    config: &ProviderConfig,
) -> Option<&'static crate::ProviderPluginDescriptor> {
    let registry = crate::builtin_registry();
    let provider = provider.trim();
    if provider.is_empty() {
        return None;
    }

    if let Some(plugin) = registry.plugin(provider) {
        return Some(plugin);
    }

    if let Some(canonical) = canonical_builder_provider_from_hint(provider) {
        if canonical == "openai-compatible" {
            return registry
                .plugin(canonical)
                .or_else(|| registry.plugin("openai"));
        }
        return registry.plugin(canonical);
    }

    if let Some(plugin) = builder_plugin_from_upstream_api(config) {
        return Some(plugin);
    }

    registry
        .plugin("openai-compatible")
        .or_else(|| registry.plugin("openai"))
}

#[derive(Debug, Clone)]
struct BuilderRuntimeResolution {
    provider: &'static str,
    config: ProviderConfig,
}

const LLM_BUILDER_OPERATIONS: &[crate::OperationKind] = &[
    crate::OperationKind::CHAT_COMPLETION,
    crate::OperationKind::RESPONSE,
    crate::OperationKind::TEXT_COMPLETION,
];
const EMBEDDING_BUILDER_OPERATIONS: &[crate::OperationKind] = &[crate::OperationKind::EMBEDDING];
const MODERATION_BUILDER_OPERATIONS: &[crate::OperationKind] = &[crate::OperationKind::MODERATION];
const IMAGE_GENERATION_BUILDER_OPERATIONS: &[crate::OperationKind] =
    &[crate::OperationKind::IMAGE_GENERATION];
const IMAGE_EDIT_BUILDER_OPERATIONS: &[crate::OperationKind] = &[crate::OperationKind::IMAGE_EDIT];
const VIDEO_GENERATION_BUILDER_OPERATIONS: &[crate::OperationKind] =
    &[crate::OperationKind::VIDEO_GENERATION];
const REALTIME_BUILDER_OPERATIONS: &[crate::OperationKind] =
    &[crate::OperationKind::REALTIME_SESSION];
const AUDIO_TRANSCRIPTION_BUILDER_OPERATIONS: &[crate::OperationKind] =
    &[crate::OperationKind::AUDIO_TRANSCRIPTION];
const AUDIO_SPEECH_BUILDER_OPERATIONS: &[crate::OperationKind] =
    &[crate::OperationKind::AUDIO_SPEECH];
const BATCH_BUILDER_OPERATIONS: &[crate::OperationKind] = &[crate::OperationKind::BATCH];
const RERANK_BUILDER_OPERATIONS: &[crate::OperationKind] = &[crate::OperationKind::RERANK];

fn builder_operations_for_capability(
    capability: crate::CapabilityKind,
) -> &'static [crate::OperationKind] {
    if capability == crate::CapabilityKind::LLM {
        LLM_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::EMBEDDING {
        EMBEDDING_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::MODERATION {
        MODERATION_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::IMAGE_GENERATION {
        IMAGE_GENERATION_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::IMAGE_EDIT {
        IMAGE_EDIT_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::VIDEO_GENERATION {
        VIDEO_GENERATION_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::REALTIME {
        REALTIME_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::AUDIO_TRANSCRIPTION {
        AUDIO_TRANSCRIPTION_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::AUDIO_SPEECH {
        AUDIO_SPEECH_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::BATCH {
        BATCH_BUILDER_OPERATIONS
    } else if capability == crate::CapabilityKind::RERANK {
        RERANK_BUILDER_OPERATIONS
    } else {
        &[]
    }
}

fn apply_runtime_route_to_builder_config(
    config: &ProviderConfig,
    route: &crate::RuntimeRoute,
) -> ProviderConfig {
    let mut runtime_config = config.clone();
    runtime_config.base_url = Some(route.base_url.clone());
    runtime_config.default_model = Some(route.invocation.model.clone());
    runtime_config
}

fn default_builder_runtime(
    provider: &str,
    config: &ProviderConfig,
) -> crate::Result<BuilderRuntimeResolution> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Err(DittoError::InvalidResponse(
            "unsupported provider backend: ".to_string(),
        ));
    }

    let plugin = resolve_builder_plugin(provider, config);
    let builder_provider = plugin
        .and_then(canonical_builder_provider_from_plugin)
        .or_else(|| canonical_builder_provider_from_hint(provider))
        .ok_or_else(|| {
            DittoError::InvalidResponse(format!("unsupported provider backend: {provider}"))
        })?;

    let mut runtime_config = config.clone();
    if runtime_config.base_url.is_none() {
        if let Some(plugin) = plugin {
            runtime_config.base_url = plugin.default_base_url.map(str::to_string);
        }
    }

    Ok(BuilderRuntimeResolution {
        provider: builder_provider,
        config: runtime_config,
    })
}

fn resolve_builder_runtime_for_capability(
    provider: &str,
    config: &ProviderConfig,
    capability: crate::CapabilityKind,
) -> crate::Result<BuilderRuntimeResolution> {
    let fallback = default_builder_runtime(provider, config)?;
    let Some(plugin) = resolve_builder_plugin(provider, config) else {
        return Ok(fallback);
    };

    let requested_model = if capability == crate::CapabilityKind::BATCH {
        None
    } else {
        configured_default_model(config)
    };

    if let Some(model) = requested_model {
        let mut first_error: Option<DittoError> = None;
        let mut error_messages = Vec::<String>::new();
        for &operation in builder_operations_for_capability(capability) {
            match crate::builtin_registry().resolve_runtime_route(
                crate::RuntimeRouteRequest::new(plugin.id, Some(model), operation)
                    .with_provider_config(config)
                    .with_required_capability(capability),
            ) {
                Ok(route) => {
                    let builder_provider = crate::builtin_registry()
                        .plugin(route.invocation.provider)
                        .and_then(canonical_builder_provider_from_plugin)
                        .unwrap_or(fallback.provider);
                    return Ok(BuilderRuntimeResolution {
                        provider: builder_provider,
                        config: apply_runtime_route_to_builder_config(config, &route),
                    });
                }
                Err(err) => {
                    if first_error.is_none() {
                        first_error = Some(err);
                    } else {
                        error_messages.push(err.to_string());
                    }
                }
            }
        }

        if error_messages.is_empty() {
            return Err(first_error.expect("builder route resolution should record an error"));
        }

        let mut messages = Vec::with_capacity(error_messages.len() + 1);
        messages.push(
            first_error
                .expect("builder route resolution should record an error")
                .to_string(),
        );
        messages.extend(error_messages);
        return Err(DittoError::InvalidResponse(format!(
            "failed to resolve runtime route for provider={} model={model} capability={capability}: {}",
            plugin.id,
            messages.join("; ")
        )));
    }

    let resolution = plugin.capability_resolution(None);
    if !resolution.effective_supports(capability) {
        return Err(
            crate::ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: plugin.id.to_string(),
                model: "*".to_string(),
                capability: capability.to_string(),
            }
            .into(),
        );
    }

    Ok(fallback)
}

pub async fn build_language_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Arc<dyn LanguageModel>> {
    let runtime =
        resolve_builder_runtime_for_capability(provider, config, crate::CapabilityKind::LLM)?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Arc::new(
                    crate::OpenAITextModel::from_config(_config, _env).await?,
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
                    crate::OpenAICompatible::from_config(_config, _env).await?,
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
                    crate::Anthropic::from_config(_config, _env).await?,
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
                Ok(Arc::new(crate::Google::from_config(_config, _env).await?))
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
                Ok(Arc::new(crate::Cohere::from_config(_config, _env).await?))
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
                Ok(Arc::new(crate::Bedrock::from_config(_config, _env).await?))
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
                Ok(Arc::new(crate::Vertex::from_config(_config, _env).await?))
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
    _env: &Env,
) -> crate::Result<Option<Arc<dyn EmbeddingModel>>> {
    let runtime =
        resolve_builder_runtime_for_capability(provider, config, crate::CapabilityKind::EMBEDDING)?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "embeddings"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIEmbeddings::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleEmbeddings::from_config(_config, _env).await?,
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
                    crate::GoogleEmbeddings::from_config(_config, _env).await?,
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
                    crate::CohereEmbeddings::from_config(_config, _env).await?,
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

pub async fn build_moderation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn ModerationModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::MODERATION,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "moderations"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIModerations::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleModerations::from_config(_config, _env).await?,
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

pub async fn build_image_generation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn ImageGenerationModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::IMAGE_GENERATION,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIImages::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleImages::from_config(_config, _env).await?,
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
                    crate::GoogleImages::from_config(_config, _env).await?,
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

pub async fn build_image_edit_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn crate::image_edit::ImageEditModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::IMAGE_EDIT,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "images"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIImageEdits::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleImageEdits::from_config(_config, _env).await?,
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

pub async fn build_video_generation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn crate::video::VideoGenerationModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::VIDEO_GENERATION,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "videos"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIVideos::from_config(_config, _env).await?,
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
                    crate::GoogleVideos::from_config(_config, _env).await?,
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

pub async fn build_realtime_session_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn crate::realtime::RealtimeSessionModel>>> {
    let runtime =
        resolve_builder_runtime_for_capability(provider, config, crate::CapabilityKind::REALTIME)?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "realtime"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIRealtime::from_config(_config, _env).await?,
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
                    crate::GoogleRealtime::from_config(_config, _env).await?,
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

pub async fn build_audio_transcription_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn AudioTranscriptionModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::AUDIO_TRANSCRIPTION,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIAudioTranscription::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleAudioTranscription::from_config(_config, _env).await?,
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

pub async fn build_speech_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn SpeechModel>>> {
    let runtime = resolve_builder_runtime_for_capability(
        provider,
        config,
        crate::CapabilityKind::AUDIO_SPEECH,
    )?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "audio"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAISpeech::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleSpeech::from_config(_config, _env).await?,
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

pub async fn build_batch_client(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn BatchClient>>> {
    let runtime =
        resolve_builder_runtime_for_capability(provider, config, crate::CapabilityKind::BATCH)?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "openai" => {
            #[cfg(all(feature = "openai", feature = "batches"))]
            {
                Ok(Some(Arc::new(
                    crate::OpenAIBatches::from_config(_config, _env).await?,
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
                    crate::OpenAICompatibleBatches::from_config(_config, _env).await?,
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

pub async fn build_rerank_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn RerankModel>>> {
    let runtime =
        resolve_builder_runtime_for_capability(provider, config, crate::CapabilityKind::RERANK)?;
    let provider = runtime.provider;
    let _config = &runtime.config;
    match provider {
        "cohere" => {
            #[cfg(all(feature = "cohere", feature = "rerank"))]
            {
                Ok(Some(Arc::new(
                    crate::CohereRerank::from_config(_config, _env).await?,
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

pub async fn build_file_client(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn FileClient>>> {
    let runtime = match default_builder_runtime(provider, config) {
        Ok(runtime) => runtime,
        Err(_) => return Ok(None),
    };
    let provider = runtime.provider;
    let _config = &runtime.config;

    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Some(Arc::new(
                    crate::OpenAI::from_config(_config, _env).await?,
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
                    crate::OpenAICompatible::from_config(_config, _env).await?,
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

#[derive(Clone)]
struct OpenAiCompatibleContextCacheAdapter {
    provider: String,
    model_id: String,
    profile: crate::ContextCacheProfile,
}

impl crate::ContextCacheModel for OpenAiCompatibleContextCacheAdapter {
    fn provider(&self) -> &str {
        self.provider.as_str()
    }

    fn model_id(&self) -> &str {
        self.model_id.as_str()
    }

    fn context_cache_profile(&self) -> &crate::ContextCacheProfile {
        &self.profile
    }
}

pub async fn build_context_cache_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> crate::Result<Option<Arc<dyn crate::ContextCacheModel>>> {
    let plugin = crate::builtin_registry()
        .plugin_for_runtime_request(provider.trim(), config.runtime_hints())
        .ok_or_else(|| {
            DittoError::InvalidResponse(format!(
                "unsupported provider backend: {}",
                provider.trim()
            ))
        })?;
    let model = configured_default_model(config).ok_or_else(|| {
        DittoError::InvalidResponse(format!(
            "context cache model is not set for provider {} (set ProviderConfig.default_model)",
            plugin.id
        ))
    })?;

    let resolution = plugin.capability_resolution(Some(model));
    if !resolution.effective_supports(crate::CapabilityKind::CONTEXT_CACHE) {
        return Err(
            crate::ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
                provider: plugin.id.to_string(),
                model: model.to_string(),
                capability: crate::CapabilityKind::CONTEXT_CACHE.to_string(),
            }
            .into(),
        );
    }

    match plugin.id {
        "deepseek" | "minimax" => {
            #[cfg(feature = "openai-compatible")]
            {
                let client = crate::OpenAICompatible::from_config(config, _env).await?;
                let profile = client.context_cache_profile();
                if !profile.supports_caching() {
                    return Err(DittoError::InvalidResponse(format!(
                        "provider {} resolved context.cache but produced an empty context cache profile",
                        plugin.id
                    )));
                }
                Ok(Some(Arc::new(OpenAiCompatibleContextCacheAdapter {
                    provider: plugin.id.to_string(),
                    model_id: client.model_id().to_string(),
                    profile,
                })))
            }
            #[cfg(not(feature = "openai-compatible"))]
            {
                Err(DittoError::InvalidResponse(
                    "ditto-llm built without openai-compatible support for context cache providers"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod model_builder_tests {
    use super::*;

    #[cfg(feature = "provider-openai")]
    #[test]
    fn builder_runtime_accepts_response_only_openai_model() {
        let runtime = resolve_builder_runtime_for_capability(
            "openai",
            &ProviderConfig {
                base_url: Some("https://api.openai.com/v1".to_string()),
                default_model: Some("computer-use-preview".to_string()),
                ..ProviderConfig::default()
            },
            crate::CapabilityKind::LLM,
        )
        .expect("response-only openai model should resolve");

        assert_eq!(runtime.provider, "openai");
        assert_eq!(
            runtime.config.default_model.as_deref(),
            Some("computer-use-preview")
        );
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
    }

    #[cfg(feature = "provider-deepseek")]
    #[test]
    fn builder_runtime_infers_deepseek_base_url_from_catalog_route() {
        let runtime = resolve_builder_runtime_for_capability(
            "deepseek",
            &ProviderConfig {
                default_model: Some("deepseek-reasoner".to_string()),
                ..ProviderConfig::default()
            },
            crate::CapabilityKind::LLM,
        )
        .expect("deepseek runtime should resolve");

        assert_eq!(runtime.provider, "openai-compatible");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            runtime.config.default_model.as_deref(),
            Some("deepseek-reasoner")
        );
    }

    #[cfg(feature = "provider-openai-compatible")]
    #[test]
    fn builder_runtime_keeps_strict_custom_provider_defaulting() {
        let runtime = resolve_builder_runtime_for_capability(
            "yunwu-openai",
            &ProviderConfig {
                base_url: Some("https://proxy.example/v1".to_string()),
                default_model: Some("chat-model".to_string()),
                ..ProviderConfig::default()
            },
            crate::CapabilityKind::LLM,
        )
        .expect("custom provider should keep generic openai-compatible runtime");

        assert_eq!(runtime.provider, "openai-compatible");
        assert_eq!(
            runtime.config.base_url.as_deref(),
            Some("https://proxy.example/v1")
        );
        assert_eq!(runtime.config.default_model.as_deref(), Some("chat-model"));
    }
}
