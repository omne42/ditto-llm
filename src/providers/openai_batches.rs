use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::openai_like;

use crate::batch::BatchClient;
use crate::profile::{Env, ProviderConfig, RequestAuth};
use crate::types::{Batch, BatchCreateRequest, BatchListResponse, BatchResponse, Warning};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAIBatches {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAIBatches {
    pub fn new(api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        let http = openai_like::default_http_client();
        let auth = openai_like::auth_from_api_key(&api_key);

        Self {
            http,
            base_url: openai_like::DEFAULT_BASE_URL.to_string(),
            auth,
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
        let auth_header = openai_like::resolve_auth_required(config, env, DEFAULT_KEYS).await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                openai_like::HTTP_TIMEOUT,
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        openai_like::apply_auth(req, self.auth.as_ref(), &self.http_query_params)
    }

    fn batches_url(&self) -> String {
        openai_like::join_endpoint(&self.base_url, "batches")
    }

    fn batch_url(&self, batch_id: &str) -> String {
        format!("{}/{batch_id}", self.batches_url())
    }

    fn batch_cancel_url(&self, batch_id: &str) -> String {
        format!("{}/cancel", self.batch_url(batch_id))
    }

    async fn parse_batch_response(&self, response: reqwest::Response) -> Result<(Batch, Value)> {
        let raw = response.json::<Value>().await?;
        let batch = serde_json::from_value::<Batch>(raw.clone())?;
        Ok((batch, raw))
    }
}

#[derive(Debug, Deserialize, Default)]
struct BatchListObject {
    #[serde(default)]
    data: Vec<Value>,
    #[serde(default)]
    has_more: Option<bool>,
    #[serde(default)]
    last_id: Option<String>,
}

#[async_trait]
impl BatchClient for OpenAIBatches {
    fn provider(&self) -> &str {
        "openai"
    }

    async fn create(&self, request: BatchCreateRequest) -> Result<BatchResponse> {
        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
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

        let url = self.batches_url();
        let response = self
            .apply_auth(self.http.post(url).json(&body))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings,
            provider_metadata: Some(raw),
        })
    }

    async fn retrieve(&self, batch_id: &str) -> Result<BatchResponse> {
        let url = self.batch_url(batch_id);
        let response = self.apply_auth(self.http.get(url)).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings: Vec::new(),
            provider_metadata: Some(raw),
        })
    }

    async fn cancel(&self, batch_id: &str) -> Result<BatchResponse> {
        let url = self.batch_cancel_url(batch_id);
        let response = self.apply_auth(self.http.post(url)).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let (batch, raw) = self.parse_batch_response(response).await?;
        Ok(BatchResponse {
            batch,
            warnings: Vec::new(),
            provider_metadata: Some(raw),
        })
    }

    async fn list(&self, limit: Option<u32>, after: Option<String>) -> Result<BatchListResponse> {
        let url = self.batches_url();
        let mut req = self.http.get(url);
        if let Some(limit) = limit {
            req = req.query(&[("limit", limit)]);
        }
        if let Some(after) = after.as_deref().filter(|s| !s.trim().is_empty()) {
            req = req.query(&[("after", after)]);
        }

        let response = self.apply_auth(req).send().await?;
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
}
