use ditto_llm::{EmbeddingModel, OpenAIEmbeddings};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".to_string())
    })?;
    let model = std::env::var("OPENAI_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let embeddings = OpenAIEmbeddings::new(api_key).with_model(model);

    let vectors = embeddings
        .embed(vec!["hello".to_string(), "world".to_string()])
        .await?;

    println!("got {} embeddings", vectors.len());
    Ok(())
}
