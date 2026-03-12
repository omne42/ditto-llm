use futures_util::StreamExt;

use ditto_llm::contracts::{Message, StreamChunk};
use ditto_llm::foundation::error::{DittoError, Result};
use ditto_llm::llm_core::model::LanguageModel;
use ditto_llm::providers::OpenAICompatible;

#[tokio::main]
async fn main() -> Result<()> {
    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
        .map_err(|_| DittoError::InvalidResponse("missing OPENAI_COMPAT_BASE_URL".to_string()))?;
    let model = std::env::var("OPENAI_COMPAT_MODEL")
        .map_err(|_| DittoError::InvalidResponse("missing OPENAI_COMPAT_MODEL".to_string()))?;
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();

    let llm = OpenAICompatible::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Stream a short poem about Rust."),
    ];

    let mut stream = llm.stream(messages.into()).await?;
    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::TextDelta { text } => print!("{text}"),
            StreamChunk::FinishReason(_) => break,
            _ => {}
        }
    }

    Ok(())
}
