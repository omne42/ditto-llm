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
        let http = default_http_client(DEFAULT_HTTP_TIMEOUT);

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
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY"];
        let auth_header = resolve_provider_request_auth_required(
            config,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
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
        let parsed =
            crate::utils::http::send_checked_json::<EmbedResponse>(req.json(&body)).await?;
        Ok(parsed.embeddings.float)
    }
}
