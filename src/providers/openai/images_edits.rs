use async_trait::async_trait;

use crate::providers::openai_images_common;
use crate::providers::openai_like;

use crate::config::{Env, ProviderConfig};
use crate::image_edit::ImageEditModel;
use crate::types::{ImageEditRequest, ImageEditResponse};
use crate::{DittoError, Result};

macro_rules! define_openai_like_image_edits {
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

            fn resolve_model<'a>(&'a self, request: &'a ImageEditRequest) -> Result<&'a str> {
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
        impl ImageEditModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn edit(&self, request: ImageEditRequest) -> Result<ImageEditResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_images_common::edit_images(self.provider(), &self.client, model, request)
                    .await
            }
        }
    };
}

#[cfg(feature = "openai")]
define_openai_like_image_edits!(
    OpenAIImageEdits,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error =
        "openai image edit model is not set (set request.model or OpenAIImageEdits::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_image_edits!(
    OpenAICompatibleImageEdits,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible image edit model is not set (set request.model or OpenAICompatibleImageEdits::with_model)",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ImageEditUpload, ImageResponseFormat, ImageSource};
    use httpmock::{Method::POST, MockServer};

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn edit_images_posts_multipart_and_parses_base64() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/images/edits")
                    .body_includes("name=\"model\"")
                    .body_includes("gpt-image-1")
                    .body_includes("name=\"prompt\"")
                    .body_includes("remove background")
                    .body_includes("name=\"image\"")
                    .body_includes("name=\"mask\"")
                    .body_includes("response_format")
                    .body_includes("b64_json");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "created": 123,
                            "data": [{
                                "b64_json": "AQID",
                                "revised_prompt": "removed background"
                            }]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIImageEdits::new("")
            .with_base_url(server.url("/v1"))
            .with_model("gpt-image-1");

        let response = client
            .edit(ImageEditRequest {
                prompt: "remove background".to_string(),
                images: vec![ImageEditUpload {
                    data: b"image-bytes".to_vec(),
                    filename: "input.png".to_string(),
                    media_type: Some("image/png".to_string()),
                }],
                mask: Some(ImageEditUpload {
                    data: b"mask-bytes".to_vec(),
                    filename: "mask.png".to_string(),
                    media_type: Some("image/png".to_string()),
                }),
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
