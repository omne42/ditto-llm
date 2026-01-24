use ditto_llm::{EmbeddingModel, OpenAICompatibleEmbeddings};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_COMPAT_BASE_URL".to_string())
    })?;
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();
    let model = std::env::var("OPENAI_COMPAT_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let embeddings = OpenAICompatibleEmbeddings::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let vectors = embeddings
        .embed(vec!["hello".to_string(), "world".to_string()])
        .await?;

    println!("got {} embeddings", vectors.len());
    Ok(())
}
