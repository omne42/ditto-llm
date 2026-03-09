use std::collections::BTreeMap;

use serde::Deserialize;

use crate::Result;

use super::auth::{HttpAuth, RequestAuth};
use super::http::apply_http_query_params;

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAiCompatibleClient {
    pub fn new(bearer_token: String, base_url: String) -> Result<Self> {
        let auth = if bearer_token.trim().is_empty() {
            None
        } else {
            Some(RequestAuth::Http(HttpAuth::bearer(&bearer_token)?))
        };
        Self::new_with_auth(auth, base_url)
    }

    pub(crate) fn new_with_auth(auth: Option<RequestAuth>, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            http,
            base_url,
            auth,
            http_query_params: BTreeMap::new(),
        })
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_http_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.http_query_params = params;
        self
    }

    fn models_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{base}/models")
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        #[derive(Debug, Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            data: Vec<ModelItem>,
        }

        #[derive(Debug, Deserialize)]
        struct ModelItem {
            id: String,
        }

        let url = self.models_url();
        let mut req = self.http.get(url);
        if let Some(auth) = self.auth.as_ref() {
            req = auth.apply(req);
        }
        req = apply_http_query_params(req, &self.http_query_params);

        let parsed = crate::utils::http::send_checked_json::<ModelsResponse>(req).await?;
        let mut out = parsed
            .data
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        Ok(out)
    }
}
