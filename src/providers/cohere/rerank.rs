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
        const DEFAULT_KEYS: &[&str] = &["COHERE_API_KEY"];
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
        let parsed =
            crate::utils::http::send_checked_json::<WireResponse>(req.json(&body)).await?;
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
