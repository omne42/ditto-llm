#[cfg(feature = "cap-embedding")]
#[derive(Clone)]
pub struct GoogleEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "cap-embedding")]
impl GoogleEmbeddings {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = default_http_client(DEFAULT_HTTP_TIMEOUT);

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("x-goog-api-key", None, &api_key)
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
        const DEFAULT_KEYS: &[&str] = &["GOOGLE_API_KEY", "GEMINI_API_KEY"];
        let auth_header = resolve_provider_request_auth_required(
            config,
            env,
            DEFAULT_KEYS,
            "x-goog-api-key",
            None,
        )
        .await?;
        let resolved =
            resolve_http_provider_config(DEFAULT_HTTP_TIMEOUT, config, Some(DEFAULT_BASE_URL))?;

        let mut out = Self::new("").with_http_client(resolved.http);
        out.auth = Some(auth_header);
        out.http_query_params = resolved.http_query_params;
        if let Some(base_url) = resolved.base_url {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = resolved.default_model {
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

    fn resolve_model(&self) -> Result<&str> {
        crate::providers::resolve_model_or_default(
            None,
            self.model.as_str(),
            "google embedding",
            "set GoogleEmbeddings::with_model",
        )
    }

    fn embed_url(&self, suffix: &str) -> String {
        let model = Google::model_path(self.model.as_str());
        http_kit::join_api_base_url_path(&self.base_url, &format!("{model}:{suffix}"))
    }
}

#[cfg(feature = "cap-embedding")]
#[derive(Debug, Deserialize)]
struct BatchEmbedResponse {
    #[serde(default)]
    embeddings: Vec<EmbeddingItem>,
}

#[cfg(feature = "cap-embedding")]
#[derive(Debug, Deserialize)]
struct SingleEmbedResponse {
    embedding: EmbeddingItem,
}

#[cfg(feature = "cap-embedding")]
#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    values: Vec<f32>,
}

#[cfg(feature = "cap-embedding")]
#[async_trait]
impl EmbeddingModel for GoogleEmbeddings {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        let _ = model;

        if texts.len() == 1 {
            let url = self.embed_url("embedContent");
            let req = self.http.post(url);
            let parsed = crate::provider_transport::send_checked_json::<SingleEmbedResponse>(
                self.apply_auth(req).json(&serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "parts": [{ "text": texts[0] }] }
                })),
            )
            .await?;
            return Ok(vec![parsed.embedding.values]);
        }

        let url = self.embed_url("batchEmbedContents");
        let requests = texts
            .into_iter()
            .map(|text| {
                serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "role": "user", "parts": [{ "text": text }] }
                })
            })
            .collect::<Vec<_>>();

        let req = self.http.post(url);
        let parsed = crate::provider_transport::send_checked_json::<BatchEmbedResponse>(
            self.apply_auth(req)
                .json(&serde_json::json!({ "requests": requests })),
        )
        .await?;
        Ok(parsed.embeddings.into_iter().map(|e| e.values).collect())
    }
}
