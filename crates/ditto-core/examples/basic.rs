use ditto_core::contracts::Message;
use ditto_core::error::Result;
use ditto_core::llm_core::model::LanguageModel;
use ditto_core::providers::OpenAICompatible;

#[tokio::main]
async fn main() -> Result<()> {
    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL").map_err(|_| {
        ditto_core::invalid_response!(
            "error_detail.freeform",
            "message" => "missing OPENAI_COMPAT_BASE_URL"
        )
    })?;
    let model = std::env::var("OPENAI_COMPAT_MODEL").map_err(|_| {
        ditto_core::invalid_response!(
            "error_detail.freeform",
            "message" => "missing OPENAI_COMPAT_MODEL"
        )
    })?;
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();

    let llm = OpenAICompatible::new(api_key)
        .with_base_url(base_url)
        .with_model(model);

    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("What is 2+2?"),
    ];

    let response = llm.generate(messages.into()).await?;
    println!("{}", response.text());

    Ok(())
}
