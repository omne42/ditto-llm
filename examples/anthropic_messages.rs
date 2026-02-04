use ditto_llm::{Anthropic, DittoError, LanguageModel, Message};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| DittoError::InvalidResponse("missing ANTHROPIC_API_KEY".to_string()))?;
    let model = std::env::var("ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-3-5-sonnet-20241022".to_string());

    let mut client = Anthropic::new(api_key).with_model(model);
    if let Ok(base_url) = std::env::var("ANTHROPIC_BASE_URL") {
        if !base_url.trim().is_empty() {
            client = client.with_base_url(base_url);
        }
    }

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Reply with the single word: ok"),
    ];

    let response = client.generate(messages.into()).await?;
    println!("{}", response.text());
    if !response.warnings.is_empty() {
        eprintln!("warnings: {:?}", response.warnings);
    }

    Ok(())
}
