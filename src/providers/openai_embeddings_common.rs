use serde::Deserialize;

use super::openai_like::OpenAiLikeClient;

use crate::Result;

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
    let parsed = crate::utils::http::send_checked_json::<EmbeddingsResponse>(
        client
            .apply_auth(client.http.post(url))
            .json(&serde_json::json!({ "model": model, "input": texts })),
    )
    .await?;
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
}
