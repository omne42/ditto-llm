#![cfg(all(
    feature = "gateway",
    feature = "gateway-translation",
    feature = "openai-compatible"
))]

use ditto_core::config::{Env, ProviderConfig};
use ditto_core::runtime::{
    build_audio_transcription_model, build_batch_client, build_embedding_model, build_file_client,
    build_image_generation_model, build_language_model, build_moderation_model, build_speech_model,
};

#[tokio::test]
async fn generic_openai_compatible_provider_aliases_build_models() -> ditto_core::error::Result<()>
{
    let env = Env::default();
    let config = ProviderConfig::default();

    // Dedicated provider packs such as deepseek/openrouter/xai have their own catalog truth and
    // separate capability tests. This list stays focused on aliases that intentionally fall back to
    // the generic OpenAI-compatible runtime.
    let aliases = [
        "openai-compatible",
        "openai_compatible",
        "litellm",
        "azure",
        "azure-openai",
        "azure_openai",
        "qwen",
        "groq",
        "mistral",
        "together",
        "together-ai",
        "together_ai",
        "fireworks",
        "perplexity",
        "ollama",
    ];

    for alias in aliases {
        let model = build_language_model(alias, &config, &env).await?;
        assert_eq!(model.provider(), "openai-compatible");

        #[cfg(feature = "embeddings")]
        assert!(build_embedding_model(alias, &config, &env).await?.is_some());
        #[cfg(not(feature = "embeddings"))]
        assert!(build_embedding_model(alias, &config, &env).await?.is_none());

        #[cfg(feature = "images")]
        assert!(
            build_image_generation_model(alias, &config, &env)
                .await?
                .is_some()
        );
        #[cfg(not(feature = "images"))]
        assert!(
            build_image_generation_model(alias, &config, &env)
                .await?
                .is_none()
        );

        #[cfg(feature = "moderations")]
        assert!(
            build_moderation_model(alias, &config, &env)
                .await?
                .is_some()
        );
        #[cfg(not(feature = "moderations"))]
        assert!(
            build_moderation_model(alias, &config, &env)
                .await?
                .is_none()
        );

        #[cfg(feature = "audio")]
        {
            assert!(
                build_audio_transcription_model(alias, &config, &env)
                    .await?
                    .is_some()
            );
            assert!(build_speech_model(alias, &config, &env).await?.is_some());
        }
        #[cfg(not(feature = "audio"))]
        {
            assert!(
                build_audio_transcription_model(alias, &config, &env)
                    .await?
                    .is_none()
            );
            assert!(build_speech_model(alias, &config, &env).await?.is_none());
        }

        #[cfg(feature = "batches")]
        assert!(build_batch_client(alias, &config, &env).await?.is_some());
        #[cfg(not(feature = "batches"))]
        assert!(build_batch_client(alias, &config, &env).await?.is_none());

        assert!(build_file_client(alias, &config, &env).await?.is_some());
    }

    Ok(())
}
