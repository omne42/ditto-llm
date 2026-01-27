use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::moderation::ModerationModel;
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{ModerationRequest, ModerationResponse, ModerationResult, Warning};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAIModerations {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAIModerations {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build should not fail");

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::bearer(&api_key).ok().map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: "https://api.openai.com/v1".to_string(),
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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
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

    fn moderations_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/moderations") {
            base.to_string()
        } else {
            format!("{base}/moderations")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a ModerationRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai moderation model is not set (set request.model or OpenAIModerations::with_model)"
                .to_string(),
        ))
    }

    fn merge_provider_options(
        body: &mut Map<String, Value>,
        options: Option<&Value>,
        warnings: &mut Vec<Warning>,
    ) {
        let Some(options) = options else {
            return;
        };
        let Some(obj) = options.as_object() else {
            warnings.push(Warning::Unsupported {
                feature: "moderation.provider_options".to_string(),
                details: Some("expected provider_options to be a JSON object".to_string()),
            });
            return;
        };

        for (key, value) in obj {
            if body.contains_key(key) {
                warnings.push(Warning::Compatibility {
                    feature: "moderation.provider_options".to_string(),
                    details: format!("provider_options overrides {key}; ignoring override"),
                });
                continue;
            }
            body.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModerationsResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    results: Vec<ModerationsResult>,
}

#[derive(Debug, Deserialize)]
struct ModerationsResult {
    #[serde(default)]
    flagged: bool,
    #[serde(default)]
    categories: BTreeMap<String, bool>,
    #[serde(default)]
    category_scores: BTreeMap<String, f64>,
}

#[async_trait]
impl ModerationModel for OpenAIModerations {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn moderate(&self, request: ModerationRequest) -> Result<ModerationResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let selected_provider_options = crate::types::select_provider_options_value(
            request.provider_options.as_ref(),
            self.provider(),
        )?;
        let mut warnings = Vec::<Warning>::new();

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.clone()));
        body.insert("input".to_string(), serde_json::to_value(&request.input)?);

        Self::merge_provider_options(&mut body, selected_provider_options.as_ref(), &mut warnings);

        let url = self.moderations_url();
        let response = self
            .apply_auth(self.http.post(url))
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ModerationsResponse>().await?;

        let results = parsed
            .results
            .into_iter()
            .map(|result| ModerationResult {
                flagged: result.flagged,
                categories: result.categories,
                category_scores: result.category_scores,
                provider_metadata: None,
            })
            .collect();

        Ok(ModerationResponse {
            id: parsed.id,
            model: parsed.model.or(Some(model.clone())),
            results,
            warnings,
            provider_metadata: Some(serde_json::json!({ "model": model })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModerationInput, ModerationRequest};
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn moderate_posts_and_parses_results() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/moderations")
                    .body_includes("\"model\":\"omni-moderation-latest\"")
                    .body_includes("\"input\":\"hi\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "modr-123",
                            "model": "omni-moderation-latest",
                            "results": [{
                                "flagged": true,
                                "categories": { "violence": true, "hate": false },
                                "category_scores": { "violence": 0.9, "hate": 0.01 }
                            }]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIModerations::new("")
            .with_base_url(server.url("/v1"))
            .with_model("omni-moderation-latest");
        let response = client
            .moderate(ModerationRequest {
                input: ModerationInput::Text("hi".to_string()),
                model: None,
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.id.as_deref(), Some("modr-123"));
        assert_eq!(response.model.as_deref(), Some("omni-moderation-latest"));
        assert_eq!(response.results.len(), 1);
        assert!(response.results[0].flagged);
        assert_eq!(response.results[0].categories.get("violence"), Some(&true));
        assert_eq!(response.results[0].categories.get("hate"), Some(&false));
        assert_eq!(
            response.results[0].category_scores.get("violence"),
            Some(&0.9)
        );
        Ok(())
    }
}
