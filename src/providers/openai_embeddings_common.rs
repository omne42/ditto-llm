use serde::Deserialize;

use super::openai_like::OpenAiLikeClient;

use crate::{DittoError, Result};

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    #[serde(default)]
    data: Vec<EmbeddingsItem>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsItem {
    embedding: Vec<f32>,
}

pub(crate) async fn embed(
    client: &OpenAiLikeClient,
    model: &str,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>> {
    let url = client.endpoint("embeddings");
    let response = client
        .apply_auth(client.http.post(url))
        .json(&serde_json::json!({ "model": model, "input": texts }))
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let parsed = response.json::<EmbeddingsResponse>().await?;
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
}
