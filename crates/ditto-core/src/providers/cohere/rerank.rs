#[cfg(feature = "cap-rerank")]
#[derive(Clone)]
pub struct CohereRerank {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "cap-rerank")]
impl CohereRerank {
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

    fn rerank_url(&self) -> String {
        http_kit::join_api_base_url_path(&self.base_url, "rerank")
    }

    fn resolve_model<'a>(&'a self, request: &'a RerankRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref() {
            let model = model.trim();
            if !model.is_empty() {
                return Ok(model);
            }
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::provider_model_missing(
            "cohere rerank",
            "set request.model or CohereRerank::with_model",
        ))
    }
}

#[cfg(feature = "cap-rerank")]
#[derive(Debug, Deserialize, Default)]
struct CohereRerankOptions {
    #[serde(default, alias = "maxTokensPerDoc", alias = "max_tokens_per_doc")]
    max_tokens_per_doc: Option<u32>,
    #[serde(default)]
    priority: Option<u32>,
}

#[cfg(feature = "cap-rerank")]
impl CohereRerankOptions {
    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value::<Self>(value.clone()).map_err(|err| {
            crate::invalid_response!("error_detail.provider_options.invalid", "error" => err.to_string())
        })
    }
}

#[cfg(feature = "cap-rerank")]
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

        let selected_provider_options = crate::provider_options::select_provider_options_value(
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
            crate::provider_transport::send_checked_json::<WireResponse>(req.json(&body)).await?;
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
