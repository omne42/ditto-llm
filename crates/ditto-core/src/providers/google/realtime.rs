#[cfg(feature = "cap-realtime")]
mod google_realtime_impl {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use reqwest::Url;

    use super::Google;
    use crate::capabilities::realtime::{
        RealtimeSessionConnection, RealtimeSessionModel, RealtimeSessionRequest,
    };
    use crate::config::{Env, ProviderConfig, RequestAuth};
    use crate::error::{DittoError, Result};

    #[derive(Clone)]
    pub struct GoogleRealtime {
        client: Google,
    }

    impl GoogleRealtime {
        pub fn new(api_key: impl Into<String>) -> Self {
            Self {
                client: Google::new(api_key),
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
            Ok(Self {
                client: Google::from_config(config, env).await?,
            })
        }

        fn resolve_model<'a>(&'a self, request: &'a RealtimeSessionRequest) -> Result<&'a str> {
            if let Some(model) = request
                .model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
            {
                return Ok(model);
            }
            if !self.client.default_model.trim().is_empty() {
                return Ok(self.client.default_model.as_str());
            }
            Err(DittoError::provider_model_missing(
                "google realtime",
                "set request.model or GoogleRealtime::with_model",
            ))
        }

        fn websocket_root_and_version(&self) -> Result<(String, String)> {
            let websocket_base =
                crate::session_transport::to_websocket_base_url(&self.client.base_url);
            let mut url = Url::parse(&websocket_base).map_err(|err| {
                DittoError::provider_base_url_invalid(
                    "google realtime",
                    self.client.base_url.as_str(),
                    err,
                )
            })?;
            let mut segments = url
                .path_segments()
                .map(|segments| {
                    segments
                        .filter(|segment| !segment.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let version = segments
                .last()
                .filter(|segment| segment.starts_with('v'))
                .map(|segment| (*segment).to_string())
                .unwrap_or_else(|| "v1beta".to_string());
            if segments
                .last()
                .is_some_and(|segment| segment.starts_with('v'))
            {
                segments.pop();
            }
            let new_path = if segments.is_empty() {
                "/".to_string()
            } else {
                format!("/{}/", segments.join("/"))
            };
            url.set_path(&new_path);
            url.set_query(None);
            url.set_fragment(None);
            Ok((url.to_string().trim_end_matches('/').to_string(), version))
        }
    }

    #[async_trait]
    impl RealtimeSessionModel for GoogleRealtime {
        fn provider(&self) -> &str {
            "google"
        }

        fn model_id(&self) -> &str {
            self.client.default_model.as_str()
        }

        async fn prepare_session(
            &self,
            request: RealtimeSessionRequest,
        ) -> Result<RealtimeSessionConnection> {
            let model = self.resolve_model(&request)?.to_string();
            let (root, version) = self.websocket_root_and_version()?;
            let mut headers = BTreeMap::new();
            let mut query_params = self.client.http_query_params.clone();
            query_params.extend(request.query_params);

            if let Some(auth) = self.client.auth.as_ref() {
                match auth {
                    RequestAuth::Http(http) => {
                        let value = http.value.to_str().map_err(|err| {
                            crate::invalid_response!(
                                "error_detail.auth.header_value_invalid",
                                "header" => http.header.as_str(),
                                "error" => err.to_string()
                            )
                        })?;
                        headers.insert(http.header.as_str().to_string(), value.to_string());
                    }
                    RequestAuth::QueryParam(query) => {
                        query_params.insert(query.param.clone(), query.value.clone());
                    }
                }
            }

            let url = http_kit::join_api_base_url_path(
                &root,
                &format!(
                    "ws/google.ai.generativelanguage.{version}.GenerativeService.BidiGenerateContent"
                ),
            );
            let query_params = query_params.into_iter().collect::<Vec<_>>();
            let url = http_kit::append_url_query_params_encoded(&url, &query_params).map_err(
                |err| {
                    crate::invalid_response!(
                        "error_detail.provider.base_url_invalid",
                        "subject" => "google realtime websocket",
                        "base_url" => format!("{root} (version={version})"),
                        "error" => err.to_string()
                    )
                },
            )?;

            Ok(RealtimeSessionConnection {
                url,
                headers,
                setup_payload: Some(serde_json::json!({
                    "setup": {
                        "model": Google::model_path(&model)
                    }
                })),
                provider_metadata: Some(serde_json::json!({
                    "provider": self.provider(),
                    "model": model,
                    "transport": "websocket",
                    "api_version": version
                })),
            })
        }
    }

    #[cfg(test)]
    mod google_realtime_tests {
        use super::*;

        #[tokio::test]
        async fn prepare_session_builds_google_live_websocket_url_and_setup_payload() -> Result<()>
        {
            let client = GoogleRealtime::new("test-google-key")
                .with_base_url("https://generativelanguage.googleapis.com/v1beta")
                .with_model("gemini-2.5-flash-live");

            let session = client
                .prepare_session(RealtimeSessionRequest::default())
                .await?;

            assert_eq!(
                session.url,
                "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
            );
            assert_eq!(
                session.headers.get("x-goog-api-key").map(String::as_str),
                Some("test-google-key")
            );
            assert_eq!(
                session.setup_payload,
                Some(serde_json::json!({
                    "setup": {
                        "model": "models/gemini-2.5-flash-live"
                    }
                }))
            );
            Ok(())
        }

        #[tokio::test]
        async fn prepare_session_encodes_query_auth_and_extra_params() -> Result<()> {
            let config = ProviderConfig {
                base_url: Some("https://proxy.example/v1beta".to_string()),
                default_model: Some("gemini-2.5-flash-live".to_string()),
                auth: Some(crate::config::ProviderAuth::QueryParamEnv {
                    param: "api_key".to_string(),
                    keys: vec!["DITTO_TEST_GOOGLE_KEY".to_string()],
                    prefix: None,
                }),
                http_query_params: BTreeMap::from([(
                    "region".to_string(),
                    "us east".to_string(),
                )]),
                ..ProviderConfig::default()
            };
            let env = Env::parse_dotenv("DITTO_TEST_GOOGLE_KEY=sk/test\n");
            let client = GoogleRealtime::from_config(&config, &env).await?;

            let session = client
                .prepare_session(RealtimeSessionRequest {
                    model: None,
                    query_params: BTreeMap::from([("trace".to_string(), "1/2".to_string())]),
                })
                .await?;

            assert_eq!(
                session.url,
                "wss://proxy.example/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?api_key=sk%2Ftest&region=us+east&trace=1%2F2"
            );
            Ok(())
        }
    }
}

#[cfg(feature = "cap-realtime")]
pub use google_realtime_impl::GoogleRealtime;
