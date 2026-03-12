#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use ditto_llm::config::{Env, ProviderApi, ProviderConfig};
use ditto_llm::runtime::{build_embedding_model, build_language_model};

fn mixed_env() -> Env {
    Env::parse_dotenv(
        "OPENAI_COMPAT_API_KEY=sk-openai-compat\nGOOGLE_API_KEY=test-google-key\nANTHROPIC_API_KEY=sk-anthropic\n",
    )
}

#[cfg(all(feature = "provider-openai-compatible", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_defaults_to_generic_openai_compatible_runtime()
-> ditto_llm::foundation::error::Result<()> {
    let model = build_language_model(
        "yunwu-openai",
        &ProviderConfig {
            base_url: Some("https://proxy.example/v1".to_string()),
            default_model: Some("chat-model".to_string()),
            ..ProviderConfig::default()
        },
        &mixed_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai-compatible");
    assert_eq!(model.model_id(), "chat-model");
    Ok(())
}

#[cfg(all(feature = "provider-openai-compatible", feature = "cap-llm"))]
#[tokio::test]
async fn custom_provider_respects_openai_upstream_api_runtime_selection()
-> ditto_llm::foundation::error::Result<()> {
    let model = build_language_model(
        "yunwu-openai-chat",
        &ProviderConfig {
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
async fn custom_provider_respects_anthropic_upstream_api_runtime_selection()
-> ditto_llm::foundation::error::Result<()> {
    let model = build_language_model(
        "yunwu-claude-native",
        &ProviderConfig {
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
async fn custom_provider_respects_google_upstream_api_runtime_selection()
-> ditto_llm::foundation::error::Result<()> {
    let model = build_language_model(
        "yunwu-gemini-native",
        &ProviderConfig {
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
async fn custom_provider_respects_google_upstream_api_for_embeddings()
-> ditto_llm::foundation::error::Result<()> {
    let model = build_embedding_model(
        "yunwu-gemini-embed",
        &ProviderConfig {
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
