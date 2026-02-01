use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;

use crate::{DittoError, Result};

use super::auth::{HttpAuth, RequestAuth, resolve_request_auth_with_default_keys};
use super::config::{ProviderCapabilities, ProviderConfig, filter_models_whitelist};
use super::env::Env;
use super::http::build_http_client;
use super::openai_compatible::OpenAiCompatibleClient;

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn list_models(&self) -> Result<Vec<String>>;
}

/// OpenAI-compatible `/models` discovery provider.
///
/// This is used for model listing/routing and does not implement text generation.
pub struct OpenAiModelsProvider {
    name: String,
    base_url: String,
    auth: Option<RequestAuth>,
    model_whitelist: Vec<String>,
    capabilities: ProviderCapabilities,
    http: reqwest::Client,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAiModelsProvider {
    pub async fn from_config(
        name: impl Into<String>,
        config: &ProviderConfig,
        env: &Env,
    ) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "OPENAI_COMPAT_API_KEY"];

        let base_url = config.base_url.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider base_url is missing".to_string())
        })?;
        let auth = match config.auth.clone() {
            Some(auth) => Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    DEFAULT_KEYS,
                    "authorization",
                    Some("Bearer "),
                )
                .await?,
            ),
            None => match DEFAULT_KEYS.iter().find_map(|key| env.get(key)) {
                Some(token) if token.trim().is_empty() => None,
                Some(token) => Some(RequestAuth::Http(HttpAuth::bearer(&token)?)),
                None => None,
            },
        };

        let http = build_http_client(Duration::from_secs(300), &config.http_headers)?;

        Ok(Self {
            name: name.into(),
            base_url: base_url.to_string(),
            auth,
            model_whitelist: config.model_whitelist.clone(),
            capabilities: config
                .capabilities
                .unwrap_or_else(ProviderCapabilities::openai_responses),
            http,
            http_query_params: config.http_query_params.clone(),
        })
    }
}

#[async_trait]
impl Provider for OpenAiModelsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let client =
            OpenAiCompatibleClient::new_with_auth(self.auth.clone(), self.base_url.clone())?
                .with_http_query_params(self.http_query_params.clone())
                .with_http_client(self.http.clone());
        let models = client.list_models().await?;
        Ok(filter_models_whitelist(models, &self.model_whitelist))
    }
}

pub async fn list_available_models(provider: &ProviderConfig, env: &Env) -> Result<Vec<String>> {
    const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "OPENAI_COMPAT_API_KEY"];

    let base_url = provider
        .base_url
        .as_deref()
        .ok_or_else(|| DittoError::InvalidResponse("provider base_url is missing".to_string()))?;
    let auth = match provider.auth.clone() {
        Some(auth) => Some(
            resolve_request_auth_with_default_keys(
                &auth,
                env,
                DEFAULT_KEYS,
                "authorization",
                Some("Bearer "),
            )
            .await?,
        ),
        None => match DEFAULT_KEYS.iter().find_map(|key| env.get(key)) {
            Some(token) if token.trim().is_empty() => None,
            Some(token) => Some(RequestAuth::Http(HttpAuth::bearer(&token)?)),
            None => None,
        },
    };
    let http = build_http_client(Duration::from_secs(300), &provider.http_headers)?;
    let client = OpenAiCompatibleClient::new_with_auth(auth, base_url.to_string())?
        .with_http_query_params(provider.http_query_params.clone())
        .with_http_client(http);
    let models = client.list_models().await?;
    Ok(filter_models_whitelist(models, &provider.model_whitelist))
}
