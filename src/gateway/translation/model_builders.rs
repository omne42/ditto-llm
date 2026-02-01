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

pub async fn build_file_client(
    provider: &str,
    config: &ProviderConfig,
    env: &Env,
) -> crate::Result<Option<Arc<dyn FileClient>>> {
    let _ = (config, env);
    let provider = provider.trim();

    match provider {
        "openai" => {
            #[cfg(feature = "openai")]
            {
                Ok(Some(Arc::new(crate::OpenAI::from_config(config, env).await?)))
            }
            #[cfg(not(feature = "openai"))]
            {
                Ok(None)
            }
        }
        "openai-compatible" | "openai_compatible" | "litellm" | "azure" | "azure-openai"
        | "azure_openai" | "deepseek" | "qwen" | "groq" | "mistral" | "together"
        | "together-ai" | "together_ai" | "fireworks" | "xai" | "perplexity" | "openrouter"
        | "ollama" => {
            #[cfg(feature = "openai-compatible")]
            {
                Ok(Some(Arc::new(
                    crate::OpenAICompatible::from_config(config, env).await?,
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
