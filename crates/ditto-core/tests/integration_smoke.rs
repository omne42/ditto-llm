#![cfg(feature = "integration")]

#[cfg(feature = "cap-embedding")]
use ditto_core::capabilities::EmbeddingModel;
use ditto_core::contracts::{GenerateRequest, Message};
use ditto_core::error::Result;
use ditto_core::llm_core::model::LanguageModel;

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(feature = "provider-openai")]
#[tokio::test]
async fn openai_generate_smoke() -> Result<()> {
    let api_key = env_nonempty("OPENAI_API_KEY");
    let model = env_nonempty("OPENAI_MODEL");
    let (Some(api_key), Some(model)) = (api_key, model) else {
        return Ok(());
    };

    let client = ditto_core::providers::OpenAI::new(api_key).with_model(model);
    let mut request: GenerateRequest = vec![
        Message::system("You are a minimal integration test."),
        Message::user("Reply with the single word: ok"),
    ]
    .into();
    request.max_tokens = Some(16);
    request.temperature = Some(0.0);

    let response = client.generate(request).await?;
    assert!(!response.text().trim().is_empty());
    Ok(())
}

#[cfg(feature = "provider-openai-compatible")]
#[tokio::test]
async fn openai_compatible_generate_smoke() -> Result<()> {
    let base_url = env_nonempty("OPENAI_COMPAT_BASE_URL");
    let model = env_nonempty("OPENAI_COMPAT_MODEL");
    let (Some(base_url), Some(model)) = (base_url, model) else {
        return Ok(());
    };

    let api_key = env_nonempty("OPENAI_COMPAT_API_KEY").unwrap_or_default();
    let client = ditto_core::providers::OpenAICompatible::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let mut request: GenerateRequest = vec![
        Message::system("You are a minimal integration test."),
        Message::user("Reply with the single word: ok"),
    ]
    .into();
    request.max_tokens = Some(16);
    request.temperature = Some(0.0);

    let response = client.generate(request).await?;
    assert!(!response.text().trim().is_empty());
    Ok(())
}

#[cfg(all(feature = "provider-openai", feature = "cap-embedding"))]
#[tokio::test]
async fn openai_embeddings_smoke() -> Result<()> {
    let api_key = env_nonempty("OPENAI_API_KEY");
    let model = env_nonempty("OPENAI_EMBEDDING_MODEL");
    let (Some(api_key), Some(model)) = (api_key, model) else {
        return Ok(());
    };

    let client = ditto_core::providers::OpenAIEmbeddings::new(api_key).with_model(model);
    let vector = client.embed_single("ok".to_string()).await?;
    assert!(!vector.is_empty());
    Ok(())
}

#[cfg(all(feature = "provider-openai-compatible", feature = "cap-embedding"))]
#[tokio::test]
async fn openai_compatible_embeddings_smoke() -> Result<()> {
    let base_url = env_nonempty("OPENAI_COMPAT_BASE_URL");
    let model = env_nonempty("OPENAI_COMPAT_EMBEDDING_MODEL");
    let (Some(base_url), Some(model)) = (base_url, model) else {
        return Ok(());
    };

    let api_key = env_nonempty("OPENAI_COMPAT_API_KEY").unwrap_or_default();

    let client = ditto_core::providers::OpenAICompatibleEmbeddings::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let vector = client.embed_single("ok".to_string()).await?;
    assert!(!vector.is_empty());
    Ok(())
}
