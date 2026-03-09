use async_trait::async_trait;

use super::openai_like;
use super::openai_moderations_common;

use crate::moderation::ModerationModel;
use crate::profile::{Env, ProviderConfig};
use crate::types::{ModerationRequest, ModerationResponse};
use crate::{DittoError, Result};

macro_rules! define_openai_like_moderations {
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

            fn resolve_model<'a>(&'a self, request: &'a ModerationRequest) -> Result<&'a str> {
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
        impl ModerationModel for $name {
            fn provider(&self) -> &str {
                $provider
            }

            fn model_id(&self) -> &str {
                self.client.model.as_str()
            }

            async fn moderate(&self, request: ModerationRequest) -> Result<ModerationResponse> {
                let model = self.resolve_model(&request)?.to_string();
                openai_moderations_common::moderate(self.provider(), &self.client, model, request)
                    .await
            }
        }
    };
}

#[cfg(feature = "openai")]
define_openai_like_moderations!(
    OpenAIModerations,
    provider = "openai",
    default_keys = &["OPENAI_API_KEY"],
    from_config = openai_like::OpenAiLikeClient::from_config_required,
    missing_model_error =
        "openai moderation model is not set (set request.model or OpenAIModerations::with_model)",
);

#[cfg(feature = "openai-compatible")]
define_openai_like_moderations!(
    OpenAICompatibleModerations,
    provider = "openai-compatible",
    default_keys = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY",],
    from_config = openai_like::OpenAiLikeClient::from_config_optional,
    missing_model_error = "openai-compatible moderation model is not set (set request.model or OpenAICompatibleModerations::with_model)",
);

#[cfg(test)]
mod tests {
    #[cfg(feature = "openai")]
    mod openai {
        use super::super::{OpenAIModerations, Result};
        use crate::moderation::ModerationModel;
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

    #[cfg(feature = "openai-compatible")]
    mod openai_compatible {
        use super::super::{OpenAICompatibleModerations, Result};
        use crate::moderation::ModerationModel;
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
                                    "flagged": false,
                                    "categories": { "violence": false },
                                    "category_scores": { "violence": 0.02 }
                                }]
                            })
                            .to_string(),
                        );
                })
                .await;

            let client = OpenAICompatibleModerations::new("")
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
            assert!(!response.results[0].flagged);
            Ok(())
        }
    }
}
