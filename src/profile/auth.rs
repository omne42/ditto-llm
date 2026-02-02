use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;

use crate::secrets::resolve_secret_string;
use crate::{DittoError, Result};

use super::config::ProviderAuth;
use super::env::Env;

#[derive(Clone)]
pub(crate) struct HttpAuth {
    pub(crate) header: HeaderName,
    pub(crate) value: HeaderValue,
}

impl std::fmt::Debug for HttpAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAuth")
            .field("header", &self.header)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl HttpAuth {
    pub(crate) fn bearer(token: &str) -> Result<Self> {
        Self::header_value("authorization", Some("Bearer "), token)
    }

    pub(crate) fn header_value(header: &str, prefix: Option<&str>, token: &str) -> Result<Self> {
        let header = header.trim();
        if header.is_empty() {
            return Err(DittoError::InvalidResponse(
                "auth header name must be non-empty".to_string(),
            ));
        }

        let header = HeaderName::from_bytes(header.as_bytes()).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid auth header name {header:?}: {err}"))
        })?;

        let mut out = String::new();
        if let Some(prefix) = prefix {
            out.push_str(prefix);
        }
        out.push_str(token);
        let mut value = HeaderValue::from_str(&out).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid auth header value for {header:?}: {err}"))
        })?;
        value.set_sensitive(true);

        Ok(Self { header, value })
    }

    pub(crate) fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header(self.header.clone(), self.value.clone())
    }
}

#[derive(Clone)]
pub(crate) struct QueryParamAuth {
    pub(crate) param: String,
    pub(crate) value: String,
}

impl std::fmt::Debug for QueryParamAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryParamAuth")
            .field("param", &self.param)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl QueryParamAuth {
    pub(crate) fn new(param: &str, prefix: Option<&str>, token: &str) -> Result<Self> {
        let param = param.trim();
        if param.is_empty() {
            return Err(DittoError::InvalidResponse(
                "auth query param name must be non-empty".to_string(),
            ));
        }

        let mut value = String::new();
        if let Some(prefix) = prefix {
            value.push_str(prefix);
        }
        value.push_str(token);

        Ok(Self {
            param: param.to_string(),
            value,
        })
    }

    pub(crate) fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.query(&[(self.param.as_str(), self.value.as_str())])
    }
}

#[derive(Clone)]
pub(crate) enum RequestAuth {
    Http(HttpAuth),
    QueryParam(QueryParamAuth),
}

impl RequestAuth {
    pub(crate) fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Http(auth) => auth.apply(req),
            Self::QueryParam(auth) => auth.apply(req),
        }
    }
}

pub(crate) async fn resolve_request_auth_with_default_keys(
    auth: &ProviderAuth,
    env: &Env,
    default_keys: &[&str],
    default_header: &str,
    default_prefix: Option<&str>,
) -> Result<RequestAuth> {
    let token = resolve_auth_token_with_default_keys(auth, env, default_keys).await?;

    match auth {
        ProviderAuth::HttpHeaderEnv { header, prefix, .. }
        | ProviderAuth::HttpHeaderCommand { header, prefix, .. } => Ok(RequestAuth::Http(
            HttpAuth::header_value(header.as_str(), prefix.as_deref(), &token)?,
        )),
        ProviderAuth::QueryParamEnv { param, prefix, .. }
        | ProviderAuth::QueryParamCommand { param, prefix, .. } => Ok(RequestAuth::QueryParam(
            QueryParamAuth::new(param.as_str(), prefix.as_deref(), &token)?,
        )),
        ProviderAuth::ApiKeyEnv { .. } | ProviderAuth::Command { .. } => Ok(RequestAuth::Http(
            HttpAuth::header_value(default_header, default_prefix, &token)?,
        )),
        ProviderAuth::SigV4 { .. } | ProviderAuth::OAuthClientCredentials { .. } => {
            Err(DittoError::InvalidResponse(
                "sigv4/oauth auth cannot be resolved to a static request header".to_string(),
            ))
        }
    }
}

pub async fn resolve_auth_token(auth: &ProviderAuth, env: &Env) -> Result<String> {
    const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "OPENAI_COMPAT_API_KEY"];
    resolve_auth_token_with_default_keys(auth, env, DEFAULT_KEYS).await
}

pub async fn resolve_auth_token_with_default_keys(
    auth: &ProviderAuth,
    env: &Env,
    default_keys: &[&str],
) -> Result<String> {
    match auth {
        ProviderAuth::ApiKeyEnv { keys }
        | ProviderAuth::HttpHeaderEnv { keys, .. }
        | ProviderAuth::QueryParamEnv { keys, .. } => {
            if keys.is_empty() {
                for key in default_keys {
                    if let Some(value) = env.get(key) {
                        return resolve_secret_if_needed(value, env).await;
                    }
                }
                return Err(DittoError::AuthCommand(format!(
                    "missing api key env (tried: {})",
                    default_keys.join(", ")
                )));
            }
            for key in keys {
                if let Some(value) = env.get(key.as_str()) {
                    return resolve_secret_if_needed(value, env).await;
                }
            }
            Err(DittoError::AuthCommand(format!(
                "missing api key env (tried: {})",
                keys.join(", "),
            )))
        }
        ProviderAuth::Command { command }
        | ProviderAuth::HttpHeaderCommand { command, .. }
        | ProviderAuth::QueryParamCommand { command, .. } => {
            let (program, args) = command
                .split_first()
                .ok_or_else(|| DittoError::AuthCommand("command is empty".to_string()))?;
            let output = tokio::process::Command::new(program)
                .args(args)
                .output()
                .await
                .map_err(|err| DittoError::AuthCommand(format!("spawn {program}: {err}")))?;
            if !output.status.success() {
                return Err(DittoError::AuthCommand(format!(
                    "command failed with status {}",
                    output.status
                )));
            }

            #[derive(Deserialize)]
            struct AuthCommandOutput {
                #[serde(default)]
                api_key: Option<String>,
                #[serde(default)]
                token: Option<String>,
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed = serde_json::from_str::<AuthCommandOutput>(stdout.trim())?;
            let token = parsed
                .api_key
                .or(parsed.token)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| DittoError::AuthCommand("json missing api_key/token".to_string()))?;

            resolve_secret_if_needed(token, env).await
        }
        ProviderAuth::SigV4 { .. } | ProviderAuth::OAuthClientCredentials { .. } => {
            Err(DittoError::InvalidResponse(
                "sigv4/oauth auth cannot be resolved to a token string".to_string(),
            ))
        }
    }
}

async fn resolve_secret_if_needed(raw: String, env: &Env) -> Result<String> {
    let raw = raw.trim().to_string();
    if raw.starts_with("secret://") {
        return resolve_secret_string(raw.as_str(), env).await;
    }
    Ok(raw)
}
