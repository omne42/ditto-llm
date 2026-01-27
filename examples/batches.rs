#[cfg(feature = "batches")]
use ditto_llm::{BatchClient, BatchCreateRequest, OpenAICompatible, OpenAICompatibleBatches};

#[cfg(feature = "batches")]
#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_COMPAT_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .map_err(|_| {
            ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".to_string())
        })?;

    let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let input_path = std::env::args().nth(1).ok_or_else(|| {
        ditto_llm::DittoError::InvalidResponse("usage: batches <requests.jsonl>".to_string())
    })?;

    let bytes = std::fs::read(&input_path)?;
    let filename = std::path::Path::new(&input_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("requests.jsonl")
        .to_string();

    let uploader = OpenAICompatible::new(api_key.clone()).with_base_url(base_url.clone());
    let input_file_id = uploader
        .upload_file_with_purpose(filename, bytes, "batch", Some("application/jsonl"))
        .await?;

    let batches = OpenAICompatibleBatches::new(api_key).with_base_url(base_url);
    let resp = batches
        .create(BatchCreateRequest {
            input_file_id,
            endpoint: "/v1/chat/completions".to_string(),
            completion_window: "24h".to_string(),
            metadata: None,
            provider_options: None,
        })
        .await?;

    println!("batch_id={} status={:?}", resp.batch.id, resp.batch.status);
    Ok(())
}

#[cfg(not(feature = "batches"))]
fn main() {
    eprintln!("This example requires `--features batches`.");
}
