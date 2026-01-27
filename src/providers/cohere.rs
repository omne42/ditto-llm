use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
#[cfg(feature = "rerank")]
use crate::rerank::RerankModel;
use crate::types::{RerankDocument, RerankRequest, RerankResponse, RerankResult, Warning};
use crate::{DittoError, Result};

const DEFAULT_BASE_URL: &str = "https://api.cohere.com/v2";

#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct CohereEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "embeddings")]
impl CohereEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("authorization", Some("Bearer "), &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            model: String::new(),
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

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY", "CODE_PM_COHERE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn embed_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/embed") {
            base.to_string()
        } else {
            format!("{base}/embed")
        }
    }

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "cohere embedding model is not set (set CohereEmbeddings::with_model)".to_string(),
        ))
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for CohereEmbeddings {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        #[derive(Debug, Deserialize)]
        struct EmbedResponse {
            embeddings: EmbedEmbeddings,
        }

        #[derive(Debug, Deserialize)]
        struct EmbedEmbeddings {
            float: Vec<Vec<f32>>,
        }

        let model = self.resolve_model()?;
        let url = self.embed_url();
        let body = json!({
            "model": model,
            "embedding_types": ["float"],
            "texts": texts,
            "input_type": "search_query",
        });

        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<EmbedResponse>().await?;
        Ok(parsed.embeddings.float)
    }
}

#[cfg(feature = "rerank")]
#[derive(Clone)]
pub struct CohereRerank {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "rerank")]
impl CohereRerank {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("authorization", Some("Bearer "), &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            default_model: String::new(),
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

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY", "CODE_PM_COHERE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn rerank_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/rerank") {
            base.to_string()
        } else {
            format!("{base}/rerank")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a RerankRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "cohere rerank model is not set (set request.model or CohereRerank::with_model)"
                .to_string(),
        ))
    }
}

#[cfg(feature = "rerank")]
#[derive(Debug, Deserialize, Default)]
struct CohereRerankOptions {
    #[serde(default, alias = "maxTokensPerDoc", alias = "max_tokens_per_doc")]
    max_tokens_per_doc: Option<u32>,
    #[serde(default)]
    priority: Option<u32>,
}

#[cfg(feature = "rerank")]
impl CohereRerankOptions {
    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value::<Self>(value.clone()).map_err(|err| {
            DittoError::InvalidResponse(format!(
                "invalid provider_options for cohere rerank: {err}"
            ))
        })
    }
}

#[cfg(feature = "rerank")]
#[async_trait]
impl RerankModel for CohereRerank {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
    }

    async fn rerank(&self, request: RerankRequest) -> Result<RerankResponse> {
        #[derive(Debug, Deserialize)]
        struct WireResponse {
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            results: Vec<WireResult>,
            #[serde(default)]
            meta: Value,
        }

        #[derive(Debug, Deserialize)]
        struct WireResult {
            index: u32,
            relevance_score: f32,
        }

        let model = self.resolve_model(&request)?.to_string();

        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let options = selected_provider_options
            .as_ref()
            .map(CohereRerankOptions::from_value)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        let has_object_documents = request
            .documents
            .iter()
            .any(|doc| !matches!(doc, RerankDocument::Text(_)));
        if has_object_documents {
            warnings.push(Warning::Compatibility {
                feature: "cohere.rerank.object_documents".to_string(),
                details: "object documents are converted to strings".to_string(),
            });
        }

        let documents = request
            .documents
            .into_iter()
            .map(|doc| match doc {
                RerankDocument::Text(text) => Ok(text),
                RerankDocument::Json(value) => Ok(serde_json::to_string(&value)?),
            })
            .collect::<Result<Vec<_>>>()?;

        let url = self.rerank_url();
        let body = json!({
            "model": model,
            "query": request.query,
            "documents": documents,
            "top_n": request.top_n,
            "max_tokens_per_doc": options.max_tokens_per_doc,
            "priority": options.priority,
        });

        let mut req = self.http.post(url);
        req = self.apply_auth(req);
        let response = req.json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<WireResponse>().await?;
        Ok(RerankResponse {
            ranking: parsed
                .results
                .into_iter()
                .map(|result| RerankResult {
                    index: result.index,
                    relevance_score: result.relevance_score,
                    provider_metadata: None,
                })
                .collect(),
            warnings,
            provider_metadata: Some(json!({ "id": parsed.id, "meta": parsed.meta })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_posts_and_parses_results() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .header("authorization", "Bearer sk-test")
                    .body_includes("\"model\":\"rerank-v3.5\"")
                    .body_includes("\"query\":\"hello\"")
                    .body_includes("\"top_n\":2")
                    .body_includes("\"documents\":[\"a\",\"b\"]");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [
                                { "index": 0, "relevance_score": 0.9 },
                                { "index": 1, "relevance_score": 0.1 }
                            ],
                            "meta": { "billed_units": { "search_units": 1 } }
                        })
                        .to_string(),
                    );
            })
            .await;

        let config = ProviderConfig {
            base_url: Some(server.url("/v2")),
            default_model: Some("rerank-v3.5".to_string()),
            auth: Some(crate::ProviderAuth::ApiKeyEnv {
                keys: vec!["CODEPM_TEST_COHERE_KEY".to_string()],
            }),
            ..ProviderConfig::default()
        };
        let env = Env {
            dotenv: BTreeMap::from([("CODEPM_TEST_COHERE_KEY".to_string(), "sk-test".to_string())]),
        };

        let client = CohereRerank::from_config(&config, &env).await?;
        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![
                    RerankDocument::Text("a".to_string()),
                    RerankDocument::Text("b".to_string()),
                ],
                model: None,
                top_n: Some(2),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.ranking.len(), 2);
        assert_eq!(response.ranking[0].index, 0);
        assert_eq!(response.ranking[0].relevance_score, 0.9);
        assert_eq!(response.ranking[1].index, 1);
        assert_eq!(response.ranking[1].relevance_score, 0.1);
        Ok(())
    }

    #[cfg(feature = "rerank")]
    #[tokio::test]
    async fn rerank_warns_on_object_documents() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v2/rerank")
                    .body_includes("\\\"answer\\\":42");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "rr-123",
                            "results": [],
                            "meta": {}
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = CohereRerank::new("sk-test")
            .with_base_url(server.url("/v2"))
            .with_model("rerank-v3.5");

        let response = client
            .rerank(RerankRequest {
                query: "hello".to_string(),
                documents: vec![RerankDocument::Json(serde_json::json!({ "answer": 42 }))],
                model: None,
                top_n: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert!(
            response
                .warnings
                .iter()
                .any(|warning| matches!(warning, Warning::Compatibility { feature, .. } if feature == "cohere.rerank.object_documents"))
        );
        Ok(())
    }
}
