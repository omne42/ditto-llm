use ditto_core::contracts::Message;
use ditto_core::error::Result;
use ditto_core::llm_core::model::LanguageModel;
use ditto_core::providers::Anthropic;

#[tokio::main]
async fn main() -> Result<()> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
        ditto_core::invalid_response!(
            "error_detail.auth.missing_api_key_env",
            "keys" => "ANTHROPIC_API_KEY"
        )
    })?;
    let model = std::env::var("ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-3-5-sonnet-20241022".to_string());

    let mut client = Anthropic::new(api_key).with_model(model);
    if let Ok(base_url) = std::env::var("ANTHROPIC_BASE_URL")
        && !base_url.trim().is_empty()
    {
        client = client.with_base_url(base_url);
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
