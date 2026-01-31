use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{DittoError, Result};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingIntensity {
    #[serde(
        alias = "none",
        alias = "disabled",
        alias = "off",
        alias = "not_supported"
    )]
    Unsupported,
    #[serde(alias = "low")]
    Small,
    #[default]
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub thinking: ThinkingIntensity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderAuth {
    #[serde(rename = "api_key_env", alias = "env", alias = "api_key")]
    ApiKeyEnv {
        #[serde(default)]
        keys: Vec<String>,
    },
    #[serde(alias = "auth_command")]
    Command { command: Vec<String> },
    #[serde(alias = "header_env")]
    HttpHeaderEnv {
        header: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "header_command")]
    HttpHeaderCommand {
        header: String,
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "query_env", alias = "query_param")]
    QueryParamEnv {
        param: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(alias = "query_command")]
    QueryParamCommand {
        param: String,
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    #[serde(rename = "sigv4", alias = "sig_v4")]
    SigV4 {
        #[serde(default)]
        access_keys: Vec<String>,
        #[serde(default)]
        secret_keys: Vec<String>,
        #[serde(default)]
        session_token_keys: Vec<String>,
        region: String,
        service: String,
    },
    #[serde(rename = "oauth_client_credentials", alias = "oauth")]
    OAuthClientCredentials {
        token_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        #[serde(default)]
        client_id_keys: Vec<String>,
        #[serde(default)]
        client_secret_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience: Option<String>,
        #[serde(default)]
        extra_params: BTreeMap<String, String>,
    },
}

impl std::fmt::Debug for ProviderAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderAuth::ApiKeyEnv { keys } => {
                f.debug_struct("ApiKeyEnv").field("keys", keys).finish()
            }
            ProviderAuth::Command { command } => {
                f.debug_struct("Command").field("command", command).finish()
            }
            ProviderAuth::HttpHeaderEnv {
                header,
                keys,
                prefix,
            } => f
                .debug_struct("HttpHeaderEnv")
                .field("header", header)
                .field("keys", keys)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::HttpHeaderCommand {
                header,
                command,
                prefix,
            } => f
                .debug_struct("HttpHeaderCommand")
                .field("header", header)
                .field("command", command)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::QueryParamEnv {
                param,
                keys,
                prefix,
            } => f
                .debug_struct("QueryParamEnv")
                .field("param", param)
                .field("keys", keys)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::QueryParamCommand {
                param,
                command,
                prefix,
            } => f
                .debug_struct("QueryParamCommand")
                .field("param", param)
                .field("command", command)
                .field("prefix", prefix)
                .finish(),
            ProviderAuth::SigV4 {
                access_keys,
                secret_keys,
                session_token_keys,
                region,
                service,
            } => f
                .debug_struct("SigV4")
                .field("access_keys", access_keys)
                .field("secret_keys", secret_keys)
                .field("session_token_keys", session_token_keys)
                .field("region", region)
                .field("service", service)
                .finish(),
            ProviderAuth::OAuthClientCredentials {
                token_url,
                client_id,
                client_secret,
                client_id_keys,
                client_secret_keys,
                scope,
                audience,
                extra_params,
            } => {
                let extra_param_keys: Vec<&str> =
                    extra_params.keys().map(|key| key.as_str()).collect();
                f.debug_struct("OAuthClientCredentials")
                    .field("token_url", token_url)
                    .field("client_id", client_id)
                    .field(
                        "client_secret",
                        &client_secret.as_ref().map(|_| "<redacted>"),
                    )
                    .field("client_id_keys", client_id_keys)
                    .field("client_secret_keys", client_secret_keys)
                    .field("scope", scope)
                    .field("audience", audience)
                    .field("extra_params", &extra_param_keys)
                    .finish()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub streaming: bool,
}

impl ProviderCapabilities {
    pub fn openai_responses() -> Self {
        Self {
            tools: true,
            vision: true,
            reasoning: true,
            json_schema: true,
            streaming: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_whitelist: Vec<String>,
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub http_query_params: BTreeMap<String, String>,
    #[serde(default)]
    pub auth: Option<ProviderAuth>,
    #[serde(default)]
    pub capabilities: Option<ProviderCapabilities>,
}

fn header_map_from_pairs(headers: &BTreeMap<String, String>) -> Result<HeaderMap> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid http header name {name:?}: {err}"))
        })?;

        let header_value = HeaderValue::from_str(value).map_err(|err| {
            DittoError::InvalidResponse(format!(
                "invalid http header value for {name:?} (value={value:?}): {err}"
            ))
        })?;

        out.insert(header_name, header_value);
    }
    Ok(out)
}

pub(crate) fn build_http_client(
    timeout: Duration,
    headers: &BTreeMap<String, String>,
) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(timeout);
    if !headers.is_empty() {
        builder = builder.default_headers(header_map_from_pairs(headers)?);
    }
    builder.build().map_err(DittoError::Http)
}

pub(crate) fn apply_http_query_params(
    mut req: reqwest::RequestBuilder,
    params: &BTreeMap<String, String>,
) -> reqwest::RequestBuilder {
    for (name, value) in params {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        req = req.query(&[(name, value)]);
    }
    req
}

fn resolve_optional_env_token(env: &Env, keys: &[&str]) -> String {
    keys.iter().find_map(|key| env.get(key)).unwrap_or_default()
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

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn list_models(&self) -> Result<Vec<String>>;
}

pub struct OpenAiProvider {
    name: String,
    base_url: String,
    auth: Option<RequestAuth>,
    model_whitelist: Vec<String>,
    capabilities: ProviderCapabilities,
    http: reqwest::Client,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAiProvider {
    pub async fn from_config(
        name: impl Into<String>,
        config: &ProviderConfig,
        env: &Env,
    ) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &[
            "OPENAI_API_KEY",
            "CODE_PM_OPENAI_API_KEY",
            "OPENAI_COMPAT_API_KEY",
        ];

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
            None => match resolve_optional_env_token(env, DEFAULT_KEYS) {
                token if token.trim().is_empty() => None,
                token => Some(RequestAuth::Http(HttpAuth::bearer(&token)?)),
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
impl Provider for OpenAiProvider {
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

#[derive(Clone, Default)]
pub struct Env {
    pub dotenv: BTreeMap<String, String>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<&str> = self.dotenv.keys().map(|key| key.as_str()).collect();
        f.debug_struct("Env").field("dotenv_keys", &keys).finish()
    }
}

impl Env {
    pub fn parse_dotenv(contents: &str) -> Self {
        Self {
            dotenv: parse_dotenv(contents),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(value) = self.dotenv.get(key) {
            return Some(value.clone());
        }
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}

pub fn parse_dotenv(contents: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }

        let mut value = raw_value.trim().to_string();
        if let Some(stripped) = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        {
            value = stripped.to_string();
        }

        if value.trim().is_empty() {
            continue;
        }

        out.insert(key.to_string(), value);
    }

    out
}

pub fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = BTreeSet::<String>::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

pub fn select_model_config<'a>(
    models: &'a BTreeMap<String, ModelConfig>,
    model: &str,
) -> Option<&'a ModelConfig> {
    if let Some(config) = models.get(model) {
        return Some(config);
    }
    models.get("*")
}

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    http_query_params: BTreeMap<String, String>,
}

impl OpenAiCompatibleClient {
    pub fn new(bearer_token: String, base_url: String) -> Result<Self> {
        let auth = if bearer_token.trim().is_empty() {
            None
        } else {
            Some(RequestAuth::Http(HttpAuth::bearer(&bearer_token)?))
        };
        Self::new_with_auth(auth, base_url)
    }

    pub(crate) fn new_with_auth(auth: Option<RequestAuth>, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            http,
            base_url,
            auth,
            http_query_params: BTreeMap::new(),
        })
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_http_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.http_query_params = params;
        self
    }

    fn models_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{base}/models")
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        #[derive(Debug, Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            data: Vec<ModelItem>,
        }

        #[derive(Debug, Deserialize)]
        struct ModelItem {
            id: String,
        }

        let url = self.models_url();
        let mut req = self.http.get(url);
        if let Some(auth) = self.auth.as_ref() {
            req = auth.apply(req);
        }
        req = apply_http_query_params(req, &self.http_query_params);
        let response = req.send().await?;

        let status = response.status();
        if !status.is_success() {
            return Err(DittoError::InvalidResponse(format!(
                "GET /models failed ({status})"
            )));
        }

        let parsed = response.json::<ModelsResponse>().await?;
        let mut out = parsed
            .data
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

pub fn filter_models_whitelist(models: Vec<String>, whitelist: &[String]) -> Vec<String> {
    if whitelist.is_empty() {
        return models;
    }

    let allow = whitelist
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect::<BTreeSet<_>>();

    models
        .into_iter()
        .filter(|m| allow.contains(m))
        .collect::<Vec<_>>()
}

pub async fn list_available_models(provider: &ProviderConfig, env: &Env) -> Result<Vec<String>> {
    const DEFAULT_KEYS: &[&str] = &[
        "OPENAI_API_KEY",
        "CODE_PM_OPENAI_API_KEY",
        "OPENAI_COMPAT_API_KEY",
    ];

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
        None => match resolve_optional_env_token(env, DEFAULT_KEYS) {
            token if token.trim().is_empty() => None,
            token => Some(RequestAuth::Http(HttpAuth::bearer(&token)?)),
        },
    };
    let http = build_http_client(Duration::from_secs(300), &provider.http_headers)?;
    let client = OpenAiCompatibleClient::new_with_auth(auth, base_url.to_string())?
        .with_http_query_params(provider.http_query_params.clone())
        .with_http_client(http);
    let models = client.list_models().await?;
    Ok(filter_models_whitelist(models, &provider.model_whitelist))
}

pub async fn resolve_auth_token(auth: &ProviderAuth, env: &Env) -> Result<String> {
    const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
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
                        return Ok(value);
                    }
                }
                return Err(DittoError::AuthCommand(format!(
                    "missing api key env (tried: {})",
                    default_keys.join(", ")
                )));
            }
            for key in keys {
                if let Some(value) = env.get(key.as_str()) {
                    return Ok(value);
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
            parsed
                .api_key
                .or(parsed.token)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| DittoError::AuthCommand("json missing api_key/token".to_string()))
        }
        ProviderAuth::SigV4 { .. } | ProviderAuth::OAuthClientCredentials { .. } => {
            Err(DittoError::InvalidResponse(
                "sigv4/oauth auth cannot be resolved to a token string".to_string(),
            ))
        }
    }
}

