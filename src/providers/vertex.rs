use std::collections::BTreeMap;

use async_trait::async_trait;
use reqwest::Url;

use crate::auth::oauth::{OAuthClientCredentials, resolve_oauth_client_credentials};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, HttpAuth, ProviderConfig};
use crate::types::{GenerateRequest, GenerateResponse};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct Vertex {
    http: reqwest::Client,
    base_url: String,
    default_model: String,
    oauth: OAuthClientCredentials,
    http_headers: BTreeMap<String, String>,
    http_query_params: BTreeMap<String, String>,
}

impl Vertex {
    pub fn new(
        oauth: OAuthClientCredentials,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(DittoError::Http)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            default_model: default_model.into(),
            oauth,
            http_headers: BTreeMap::new(),
            http_query_params: BTreeMap::new(),
        })
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_http_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.http_headers = headers;
        self
    }

    pub fn with_http_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.http_query_params = params;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        let base_url = config.base_url.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider base_url is missing".to_string())
        })?;
        let model = config.default_model.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider default_model is missing".to_string())
        })?;
        let auth = config
            .auth
            .clone()
            .ok_or_else(|| DittoError::InvalidResponse("vertex auth is missing".to_string()))?;
        let oauth = resolve_oauth_client_credentials(&auth, env)?;

        let mut out = Self::new(oauth, base_url, model)?;
        out.http_headers = config.http_headers.clone();
        out.http_query_params = config.http_query_params.clone();
        Ok(out)
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "vertex model is not set".to_string(),
        ))
    }

    fn generate_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            return self.base_url.replace("{model}", model);
        }
        if self.base_url.ends_with(":generateContent") {
            return self.base_url.clone();
        }
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/models/{model}:generateContent")
    }

    fn build_url_with_query(&self, base: &str) -> Result<String> {
        let mut url = Url::parse(base).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid vertex base_url {base:?}: {err}"))
        })?;
        if !self.http_query_params.is_empty() {
            {
                let mut pairs = url.query_pairs_mut();
                for (key, value) in &self.http_query_params {
                    if key.trim().is_empty() {
                        continue;
                    }
                    pairs.append_pair(key, value);
                }
            }
        }
        Ok(url.to_string())
    }

    async fn post_json<T: serde::Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<GenerateResponse> {
        let token = self.oauth.fetch_token(&self.http).await?;
        let auth = HttpAuth::header_value("authorization", Some("Bearer "), &token.access_token)?;

        let mut req = self.http.post(url).json(body);
        for (name, value) in &self.http_headers {
            req = req.header(name, value);
        }
        req = auth.apply(req);

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }
        let parsed = response.json::<GenerateResponse>().await?;
        Ok(parsed)
    }
}

#[async_trait]
impl LanguageModel for Vertex {
    fn provider(&self) -> &str {
        "vertex"
    }

    fn model_id(&self) -> &str {
        &self.default_model
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        let url = self.generate_url(model);
        let url = self.build_url_with_query(&url)?;
        self.post_json(&url, &request).await
    }

    async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
        Err(DittoError::InvalidResponse(
            "vertex streaming is not implemented".to_string(),
        ))
    }
}
