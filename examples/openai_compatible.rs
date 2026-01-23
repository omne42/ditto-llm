use ditto_llm::{DittoError, LanguageModel, Message, OpenAICompatible};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
        .map_err(|_| DittoError::InvalidResponse("missing OPENAI_COMPAT_BASE_URL".to_string()))?;
    let model = std::env::var("OPENAI_COMPAT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();

    let llm = OpenAICompatible::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Say hello from an OpenAI-compatible provider."),
    ];

    let response = llm.generate(messages.into()).await?;
    println!("{}", response.text());
    if !response.warnings.is_empty() {
        eprintln!("warnings: {:?}", response.warnings);
    }

    Ok(())
}
