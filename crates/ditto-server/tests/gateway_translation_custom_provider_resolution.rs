#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use ditto_core::config::{Env, ProviderApi, ProviderConfig};
#[cfg(feature = "cap-embedding")]
use ditto_core::runtime::build_embedding_model;
use ditto_core::runtime::build_language_model;

fn mixed_env() -> Env {
    Env::parse_dotenv(
        "OPENAI_COMPAT_API_KEY=sk-openai-compat\nGOOGLE_API_KEY=test-google-key\nANTHROPIC_API_KEY=sk-anthropic\n",
    )
}

#[cfg(all(feature = "provider-openai-compatible", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_requires_explicit_openai_compatible_owner() -> ditto_core::error::Result<()>
{
    let err = build_language_model(
        "yunwu-openai",
        &ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("chat-model".to_string()),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await
    .err()
    .expect("unknown custom provider should fail closed");

    assert!(
        err.to_string()
            .contains("unsupported provider backend: yunwu-openai")
    );
    Ok(())
}

#[cfg(all(feature = "provider-openai-compatible", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_resolves_through_explicit_openai_compatible_owner()
-> ditto_core::error::Result<()> {
    let model = build_language_model(
        "yunwu-openai-chat",
        &ProviderConfig {
            provider: Some("openai-compatible".to_string()),
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("chat-model".to_string()),
            upstream_api: Some(ProviderApi::OpenaiChatCompletions),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai-compatible");
    assert_eq!(model.model_id(), "chat-model");
    Ok(())
}

#[cfg(all(feature = "provider-anthropic", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_resolves_through_explicit_anthropic_owner() -> ditto_core::error::Result<()>
{
    let model = build_language_model(
        "yunwu-claude-native",
        &ProviderConfig {
            provider: Some("anthropic".to_string()),
            base_url: Some("https://api.anthropic.com/v1".to_string()),
            default_model: Some("claude-3-7-sonnet-20250219".to_string()),
            upstream_api: Some(ProviderApi::AnthropicMessages),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await?;

    assert_eq!(model.provider(), "anthropic");
    assert_eq!(model.model_id(), "claude-3-7-sonnet-20250219");
    Ok(())
}

#[cfg(all(feature = "provider-google", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_resolves_through_explicit_google_owner() -> ditto_core::error::Result<()> {
    let model = build_language_model(
        "yunwu-gemini-native",
        &ProviderConfig {
            provider: Some("google.providers.yunwu".to_string()),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
            default_model: Some("gemini-3.1-pro".to_string()),
            upstream_api: Some(ProviderApi::GeminiGenerateContent),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await?;

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "gemini-3.1-pro");
    Ok(())
}

#[cfg(all(feature = "provider-google", feature = "cap-embedding"))]
#[tokio::test]
async fn custom_provider_resolves_embeddings_through_explicit_google_owner()
-> ditto_core::error::Result<()> {
    let model = build_embedding_model(
        "yunwu-gemini-embed",
        &ProviderConfig {
            provider: Some("google.providers.yunwu".to_string()),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
            default_model: Some("gemini-embedding".to_string()),
            upstream_api: Some(ProviderApi::GeminiGenerateContent),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await?
    .expect("google embedding runtime should resolve from upstream_api");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model_id(), "gemini-embedding");
    Ok(())
}
