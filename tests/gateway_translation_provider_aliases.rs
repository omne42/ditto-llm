#![cfg(all(
    feature = "gateway",
    feature = "gateway-translation",
    feature = "openai-compatible"
))]

use ditto_llm::gateway::translation::{
    build_audio_transcription_model, build_batch_client, build_embedding_model, build_file_client,
    build_image_generation_model, build_language_model, build_moderation_model, build_speech_model,
};
use ditto_llm::{Env, ProviderConfig};

#[tokio::test]
async fn openai_compatible_provider_aliases_build_models() -> ditto_llm::Result<()> {
    let env = Env::default();
    let config = ProviderConfig::default();

    let aliases = [
        "openai-compatible",
        "openai_compatible",
        "litellm",
        "azure",
        "azure-openai",
        "azure_openai",
        "deepseek",
        "qwen",
        "groq",
        "mistral",
        "together",
        "together-ai",
        "together_ai",
        "fireworks",
        "xai",
        "perplexity",
        "openrouter",
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
