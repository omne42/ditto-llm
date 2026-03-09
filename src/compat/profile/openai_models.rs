use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::Result;

use super::auth::{RequestAuth, resolve_provider_request_auth_optional};
use super::catalog_bridge::resolve_openai_compatible_provider_capability_profile;
use super::env::Env;
use super::http::{DEFAULT_HTTP_TIMEOUT, resolve_http_provider_config};
use super::openai_compatible::OpenAiCompatibleClient;
use super::provider_config::{ProviderCapabilities, ProviderConfig, filter_models_whitelist};

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
        const DEFAULT_KEYS: &[&str] = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];

        let name = name.into();
        let capability_profile =
            resolve_openai_compatible_provider_capability_profile(name.as_str(), config)?;
        let auth = resolve_provider_request_auth_optional(
            config,
            env,
            DEFAULT_KEYS,
            "authorization",
            Some("Bearer "),
        )
        .await?;
        let resolved = resolve_http_provider_config(DEFAULT_HTTP_TIMEOUT, config, None)?;
        let base_url = resolved
            .required_base_url("provider base_url is missing")?
            .to_string();

        Ok(Self {
            name,
            base_url,
            auth,
            model_whitelist: config.model_whitelist.clone(),
            capabilities: capability_profile.effective_capabilities,
            http: resolved.http,
            http_query_params: resolved.http_query_params,
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
    const DEFAULT_KEYS: &[&str] = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];

    let auth = resolve_provider_request_auth_optional(
        provider,
        env,
        DEFAULT_KEYS,
        "authorization",
        Some("Bearer "),
    )
    .await?;
    let resolved = resolve_http_provider_config(DEFAULT_HTTP_TIMEOUT, provider, None)?;
    let base_url = resolved
        .required_base_url("provider base_url is missing")?
        .to_string();
    let client = OpenAiCompatibleClient::new_with_auth(auth, base_url)?
        .with_http_query_params(resolved.http_query_params)
        .with_http_client(resolved.http);
    let models = client.list_models().await?;
    Ok(filter_models_whitelist(models, &provider.model_whitelist))
}
