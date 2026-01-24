use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::image::ImageGenerationModel;
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth,
    resolve_request_auth_with_default_keys,
};
use crate::types::{ImageGenerationRequest, ImageGenerationResponse, ImageSource, Usage, Warning};
use crate::{DittoError, Result};

#[derive(Clone)]
pub struct OpenAIImages {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
}

impl OpenAIImages {
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
        match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        }
    }

    fn images_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/images/generations") {
            base.to_string()
        } else {
            format!("{base}/images/generations")
        }
    }

    fn resolve_model<'a>(&'a self, request: &'a ImageGenerationRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "openai image model is not set (set request.model or OpenAIImages::with_model)"
                .to_string(),
        ))
    }

    fn parse_usage(value: &Value) -> Usage {
        let mut usage = Usage::default();
        if let Some(obj) = value.as_object() {
            usage.input_tokens = obj.get("prompt_tokens").and_then(Value::as_u64);
            usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
            usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
        }
        usage.merge_total();
        usage
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
                feature: "image.provider_options".to_string(),
                details: Some("expected provider_options to be a JSON object".to_string()),
            });
            return;
        };

        for (key, value) in obj {
            if body.contains_key(key) {
                warnings.push(Warning::Compatibility {
                    feature: "image.provider_options".to_string(),
                    details: format!("provider_options overrides {key}; ignoring override"),
                });
                continue;
            }
            body.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Deserialize)]
struct ImagesGenerationResponse {
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    data: Vec<ImageGenerationData>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ImageGenerationData {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
}

#[async_trait]
impl ImageGenerationModel for OpenAIImages {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn generate(&self, request: ImageGenerationRequest) -> Result<ImageGenerationResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let mut warnings = Vec::<Warning>::new();

        let mut body = Map::<String, Value>::new();
        body.insert("model".to_string(), Value::String(model.clone()));
        body.insert("prompt".to_string(), Value::String(request.prompt));
        if let Some(n) = request.n {
            body.insert("n".to_string(), Value::Number(n.into()));
        }
        if let Some(size) = request.size.as_deref().filter(|s| !s.trim().is_empty()) {
            body.insert("size".to_string(), Value::String(size.to_string()));
        }
        if let Some(format) = request.response_format {
            body.insert("response_format".to_string(), serde_json::to_value(format)?);
        }

        Self::merge_provider_options(&mut body, request.provider_options.as_ref(), &mut warnings);

        let url = self.images_url();
        let req = self.http.post(url).json(&body);
        let response = self.apply_auth(req).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<ImagesGenerationResponse>().await?;
        let usage = parsed
            .usage
            .as_ref()
            .map(Self::parse_usage)
            .unwrap_or_default();

        let mut images = Vec::<ImageSource>::new();
        let mut revised_prompts = Vec::<String>::new();
        for item in parsed.data {
            if let Some(prompt) = item
                .revised_prompt
                .as_deref()
                .filter(|v| !v.trim().is_empty())
            {
                revised_prompts.push(prompt.to_string());
            }

            if let Some(url) = item.url.as_deref().filter(|v| !v.trim().is_empty()) {
                images.push(ImageSource::Url {
                    url: url.to_string(),
                });
                continue;
            }
            if let Some(data) = item.b64_json.as_deref().filter(|v| !v.trim().is_empty()) {
                images.push(ImageSource::Base64 {
                    media_type: "image/png".to_string(),
                    data: data.to_string(),
                });
                continue;
            }

            warnings.push(Warning::Compatibility {
                feature: "image.data".to_string(),
                details: "image item is missing both url and b64_json".to_string(),
            });
        }

        let mut provider_metadata = serde_json::json!({ "model": model });
        if let Some(created) = parsed.created {
            provider_metadata
                .as_object_mut()
                .expect("provider_metadata is object")
                .insert("created".to_string(), Value::Number(created.into()));
        }
        if !revised_prompts.is_empty() {
            provider_metadata
                .as_object_mut()
                .expect("provider_metadata is object")
                .insert(
                    "revised_prompts".to_string(),
                    Value::Array(revised_prompts.into_iter().map(Value::String).collect()),
                );
        }

        Ok(ImageGenerationResponse {
            images,
            usage,
            warnings,
            provider_metadata: Some(provider_metadata),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ImageResponseFormat;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn generate_images_supports_base64() -> Result<()> {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/images/generations")
                    .body_includes("\"model\":\"gpt-image-1\"")
                    .body_includes("\"prompt\":\"hi\"")
                    .body_includes("\"response_format\":\"b64_json\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "created": 123,
                            "data": [{
                                "b64_json": "AQID",
                                "revised_prompt": "hello"
                            }]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIImages::new("")
            .with_base_url(server.url("/v1"))
            .with_model("gpt-image-1");

        let response = client
            .generate(ImageGenerationRequest {
                prompt: "hi".to_string(),
                model: None,
                n: None,
                size: None,
                response_format: Some(ImageResponseFormat::Base64Json),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.images.len(), 1);
        match &response.images[0] {
            ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data, "AQID");
            }
            other => panic!("unexpected image source: {other:?}"),
        }

        Ok(())
    }
}
