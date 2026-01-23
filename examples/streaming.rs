use futures_util::StreamExt;

use ditto_llm::{LanguageModel, Message, OpenAI, StreamChunk};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".to_string())
    })?;
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let openai = OpenAI::new(api_key).with_model(model);

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Stream a short poem about Rust."),
    ];

    let mut stream = openai.stream(messages.into()).await?;
    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::TextDelta { text } => print!("{text}"),
            StreamChunk::FinishReason(_) => break,
            _ => {}
        }
    }

    Ok(())
}
