use async_trait::async_trait;

use super::openai_images_common;
use super::openai_like;

use crate::image::ImageGenerationModel;
use crate::profile::{Env, ProviderConfig};
use crate::types::{ImageGenerationRequest, ImageGenerationResponse};
use crate::{DittoError, Result};

macro_rules! define_openai_like_images {
    (
        $name:ident,
        provider = $provider:literal,
        default_keys = $default_keys:expr,
        from_config = $from_config:path,
        missing_model_error = $missing_model_error:literal $(,)?
    ) => {
        #[derive(Clone)]
        pub struct $name {
            client: openai_like::OpenAiLikeClient,
        }

        impl $name {
            pub fn new(api_key: impl Into<String>) -> Self {
                Self {
                    client: openai_like::OpenAiLikeClient::new(api_key),
                }
            }

            pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
                self.client = self.client.with_http_client(http);
                self
            }

            pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
                self.client = self.client.with_base_url(base_url);
                self
            }

            pub fn with_model(mut self, model: impl Into<String>) -> Self {
                self.client = self.client.with_model(model);
                self
            }

            pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
                const DEFAULT_KEYS: &[&str] = $default_keys;
                Ok(Self {
                    client: $from_config(config, env, DEFAULT_KEYS).await?,
                })
            }

            fn resolve_model<'a>(&'a self, request: &'a ImageGenerationRequest) -> Result<&'a str> {
                if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
                    return Ok(model);
                }
                if !self.client.model.trim().is_empty() {
                    return Ok(self.client.model.as_str());
                }
                Err(DittoError::InvalidResponse(
                    $missing_model_error.to_string(),
                ))
            }
        }

        #[async_trait]
        impl ImageGenerationModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn generate(
                &self,
                request: ImageGenerationRequest,
            ) -> Result<ImageGenerationResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_images_common::generate_images(self.provider(), &self.client, model, request)
                    .await
            }
        }
    };
}

define_openai_like_images!(
    OpenAIImages,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error =
        "openai image model is not set (set request.model or OpenAIImages::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_images!(
    OpenAICompatibleImages,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY",],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible image model is not set (set request.model or OpenAICompatibleImages::with_model)",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ImageResponseFormat;
    use crate::types::ImageSource;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn generate_images_supports_base64() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
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

    #[cfg(feature = "openai-compatible")]
    #[tokio::test]
    async fn generate_images_supports_url() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/images/generations")
                    .body_includes("\"model\":\"dall-e-3\"")
                    .body_includes("\"prompt\":\"hi\"")
                    .body_includes("\"response_format\":\"url\"");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "created": 123,
                            "data": [{
                                "url": "https://example.com/image.png",
                                "revised_prompt": "hello"
                            }]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAICompatibleImages::new("")
            .with_base_url(server.url("/v1"))
            .with_model("dall-e-3");

        let response = client
            .generate(ImageGenerationRequest {
                prompt: "hi".to_string(),
                model: None,
                n: None,
                size: None,
                response_format: Some(ImageResponseFormat::Url),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.images.len(), 1);
        match &response.images[0] {
            ImageSource::Url { url } => assert_eq!(url, "https://example.com/image.png"),
            other => panic!("unexpected image source: {other:?}"),
        }
        Ok(())
    }
}
