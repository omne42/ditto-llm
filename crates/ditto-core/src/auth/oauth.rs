use std::collections::BTreeMap;

pub use http_auth_kit::OAuthToken;

use crate::config::{Env, ProviderAuth};
use crate::error::{DittoError, Result};

fn oauth_field_required(field: &str) -> DittoError {
    crate::invalid_response!("error_detail.oauth.field_required", "field" => field)
}

fn oauth_expected_auth() -> DittoError {
    crate::invalid_response!("error_detail.oauth.expected_auth")
}

fn oauth_missing_field(label: &str, keys: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.oauth.missing_field",
        "label" => label,
        "keys" => keys
    )
}

fn oauth_foundation_error(error: http_auth_kit::HttpAuthError) -> DittoError {
    match error {
        http_auth_kit::HttpAuthError::FieldRequired { field } => oauth_field_required(field),
        http_auth_kit::HttpAuthError::OAuthResponseMissingAccessToken => {
            crate::invalid_response!("error_detail.oauth.response_missing_access_token")
        }
        other => crate::invalid_response!(format!("OAuth auth failed: {other}")),
    }
}

#[derive(Clone)]
pub struct OAuthClientCredentials {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: Option<String>,
    pub audience: Option<String>,
    pub extra_params: BTreeMap<String, String>,
}

impl std::fmt::Debug for OAuthClientCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let extra_param_keys: Vec<&str> = self.extra_params.keys().map(String::as_str).collect();
        f.debug_struct("OAuthClientCredentials")
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("scope", &self.scope)
            .field("audience", &self.audience)
            .field("extra_params", &extra_param_keys)
            .finish()
    }
}

impl OAuthClientCredentials {
    pub fn new(
        token_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Result<Self> {
        let inner = http_auth_kit::OAuthClientCredentials::new(
            token_url.into(),
            client_id.into(),
            client_secret.into(),
        )
        .map_err(oauth_foundation_error)?;

        Ok(Self {
            token_url: inner.token_url,
            client_id: inner.client_id,
            client_secret: inner.client_secret,
            scope: None,
            audience: None,
            extra_params: BTreeMap::new(),
        })
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    pub fn with_extra_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_params.insert(key.into(), value.into());
        self
    }

    pub async fn fetch_token(&self, http: &reqwest::Client) -> Result<OAuthToken> {
        self.to_foundation()?
            .fetch_token(http)
            .await
            .map_err(oauth_foundation_error)
    }

    fn to_foundation(&self) -> Result<http_auth_kit::OAuthClientCredentials> {
        let mut out = http_auth_kit::OAuthClientCredentials::new(
            self.token_url.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
        .map_err(oauth_foundation_error)?;
        if let Some(scope) = self.scope.as_ref() {
            out = out.with_scope(scope);
        }
        if let Some(audience) = self.audience.as_ref() {
            out = out.with_audience(audience);
        }
        for (key, value) in &self.extra_params {
            out = out.with_extra_param(key, value);
        }
        Ok(out)
    }
}

pub fn resolve_oauth_client_credentials(
    auth: &ProviderAuth,
    env: &Env,
) -> Result<OAuthClientCredentials> {
    let ProviderAuth::OAuthClientCredentials {
        token_url,
        client_id,
        client_secret,
        client_id_keys,
        client_secret_keys,
        scope,
        audience,
        extra_params,
    } = auth
    else {
        return Err(oauth_expected_auth());
    };

    let resolved_client_id = resolve_oauth_field(
        env,
        client_id.as_ref(),
        client_id_keys,
        &["OAUTH_CLIENT_ID"],
        "client_id",
    )?;
    let resolved_client_secret = resolve_oauth_field(
        env,
        client_secret.as_ref(),
        client_secret_keys,
        &["OAUTH_CLIENT_SECRET"],
        "client_secret",
    )?;

    let mut out = OAuthClientCredentials::new(
        token_url.to_string(),
        resolved_client_id,
        resolved_client_secret,
    )?;
    if let Some(scope) = scope.as_ref().filter(|s| !s.trim().is_empty()) {
        out = out.with_scope(scope);
    }
    if let Some(audience) = audience.as_ref().filter(|s| !s.trim().is_empty()) {
        out = out.with_audience(audience);
    }
    for (key, value) in extra_params {
        out = out.with_extra_param(key, value);
    }
    Ok(out)
}

fn resolve_oauth_field(
    env: &Env,
    explicit: Option<&String>,
    keys: &[String],
    defaults: &[&str],
    label: &str,
) -> Result<String> {
    if let Some(value) = explicit.filter(|value| !value.trim().is_empty()) {
        return Ok(value.to_string());
    }
    let candidate_keys: Vec<String> = if keys.is_empty() {
        defaults.iter().map(|key| key.to_string()).collect()
    } else {
        keys.to_vec()
    };

    for key in &candidate_keys {
        if let Some(value) = env.get(key.as_str()) {
            return Ok(value);
        }
    }
    Err(oauth_missing_field(label, &candidate_keys.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn fetches_oauth_token_via_http() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/token")
                    .body_includes("grant_type=client_credentials")
                    .body_includes("client_id=test-client")
                    .body_includes("client_secret=secret");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"access_token":"tok-123","token_type":"Bearer","expires_in":3600}"#);
            })
            .await;

        let http = reqwest::Client::new();
        let oauth = OAuthClientCredentials::new(server.url("/token"), "test-client", "secret")?;
        let token = oauth.fetch_token(&http).await?;
        mock.assert_async().await;

        assert_eq!(token.access_token, "tok-123");
        assert_eq!(token.token_type, "Bearer");
        Ok(())
    }

    #[test]
    fn resolves_oauth_from_provider_auth_env() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([
                ("OAUTH_CLIENT_ID".to_string(), "client-1".to_string()),
                ("OAUTH_CLIENT_SECRET".to_string(), "secret-1".to_string()),
            ]),
        };
        let auth = ProviderAuth::OAuthClientCredentials {
            token_url: "https://example.com/token".to_string(),
            client_id: None,
            client_secret: None,
            client_id_keys: Vec::new(),
            client_secret_keys: Vec::new(),
            scope: Some("scope-a".to_string()),
            audience: None,
            extra_params: BTreeMap::new(),
        };

        let resolved = resolve_oauth_client_credentials(&auth, &env)?;
        assert_eq!(resolved.client_id, "client-1");
        assert_eq!(resolved.client_secret, "secret-1");
        assert_eq!(resolved.scope.as_deref(), Some("scope-a"));
        Ok(())
    }
}
