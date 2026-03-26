use ditto_core::capabilities::EmbeddingModel;
use ditto_core::error::Result;
use ditto_core::providers::OpenAIEmbeddings;

#[tokio::main]
async fn main() -> Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_core::invalid_response!(
            "error_detail.auth.missing_api_key_env",
            "keys" => "OPENAI_API_KEY"
        )
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
