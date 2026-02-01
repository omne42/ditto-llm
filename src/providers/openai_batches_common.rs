use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::types::{Batch, BatchCreateRequest, BatchListResponse, BatchResponse, Warning};
use crate::{DittoError, Result};

#[derive(Debug, Deserialize, Default)]
struct BatchListObject {
    #[serde(default)]
    data: Vec<Value>,
    #[serde(default)]
    has_more: Option<bool>,
    #[serde(default)]
    last_id: Option<String>,
}

fn batches_url(client: &OpenAiLikeClient) -> String {
    client.endpoint("batches")
}

fn batch_url(client: &OpenAiLikeClient, batch_id: &str) -> String {
    format!("{}/{batch_id}", batches_url(client))
}

fn batch_cancel_url(client: &OpenAiLikeClient, batch_id: &str) -> String {
    format!("{}/cancel", batch_url(client, batch_id))
}

async fn parse_batch_response(response: reqwest::Response) -> Result<(Batch, Value)> {
    let raw = response.json::<Value>().await?;
    let batch = serde_json::from_value::<Batch>(raw.clone())?;
    Ok((batch, raw))
}

pub(crate) async fn create(
    provider: &str,
    client: &OpenAiLikeClient,
    request: BatchCreateRequest,
) -> Result<BatchResponse> {
    let selected_provider_options =
        crate::types::select_provider_options_value(request.provider_options.as_ref(), provider)?;
    let mut warnings = Vec::<Warning>::new();

    let mut body = Map::<String, Value>::new();
    body.insert(
        "input_file_id".to_string(),
        Value::String(request.input_file_id),
    );
    body.insert("endpoint".to_string(), Value::String(request.endpoint));
    body.insert(
        "completion_window".to_string(),
        Value::String(request.completion_window),
    );
    if let Some(metadata) = request.metadata {
        body.insert("metadata".to_string(), serde_json::to_value(metadata)?);
    }

    crate::types::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &[],
        "batches.create.provider_options",
        &mut warnings,
    );

    let url = batches_url(client);
    let response = client
        .apply_auth(client.http.post(url))
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let (batch, raw) = parse_batch_response(response).await?;
    Ok(BatchResponse {
        batch,
        warnings,
        provider_metadata: Some(raw),
    })
}

pub(crate) async fn retrieve(client: &OpenAiLikeClient, batch_id: &str) -> Result<BatchResponse> {
    let url = batch_url(client, batch_id);
    let response = client.apply_auth(client.http.get(url)).send().await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let (batch, raw) = parse_batch_response(response).await?;
    Ok(BatchResponse {
        batch,
        warnings: Vec::new(),
        provider_metadata: Some(raw),
    })
}

pub(crate) async fn cancel(client: &OpenAiLikeClient, batch_id: &str) -> Result<BatchResponse> {
    let url = batch_cancel_url(client, batch_id);
    let response = client.apply_auth(client.http.post(url)).send().await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let (batch, raw) = parse_batch_response(response).await?;
    Ok(BatchResponse {
        batch,
        warnings: Vec::new(),
        provider_metadata: Some(raw),
    })
}

pub(crate) async fn list(
    client: &OpenAiLikeClient,
    limit: Option<u32>,
    after: Option<String>,
) -> Result<BatchListResponse> {
    let url = batches_url(client);
    let mut req = client.http.get(url);
    if let Some(limit) = limit {
        req = req.query(&[("limit", limit)]);
    }
    if let Some(after) = after.as_deref().filter(|s| !s.trim().is_empty()) {
        req = req.query(&[("after", after)]);
    }

    let response = client.apply_auth(req).send().await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let raw = response.json::<Value>().await?;
    let parsed = serde_json::from_value::<BatchListObject>(raw.clone())?;
    let mut batches = Vec::<Batch>::new();
    for item in parsed.data {
        batches.push(serde_json::from_value::<Batch>(item)?);
    }

    Ok(BatchListResponse {
        batches,
        after: parsed.last_id,
        has_more: parsed.has_more,
        warnings: Vec::new(),
        provider_metadata: Some(raw),
    })
}
