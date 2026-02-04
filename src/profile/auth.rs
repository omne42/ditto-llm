use std::time::Duration;

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
            let stdout = run_auth_command(command, env).await?;
            let token = parse_auth_command_token(stdout.as_str())?;
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

const DEFAULT_AUTH_COMMAND_TIMEOUT_SECS: u64 = 15;
const MAX_AUTH_COMMAND_TIMEOUT_SECS: u64 = 300;

fn auth_command_timeout(env: &Env) -> Duration {
    let ms = env
        .get("DITTO_AUTH_COMMAND_TIMEOUT_MS")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    if let Some(ms) = ms {
        return Duration::from_millis(ms);
    }

    let secs = env
        .get("DITTO_AUTH_COMMAND_TIMEOUT_SECS")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_AUTH_COMMAND_TIMEOUT_SECS)
        .min(MAX_AUTH_COMMAND_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

async fn run_auth_command(command: &[String], env: &Env) -> Result<String> {
    use std::process::Stdio;

    let timeout = auth_command_timeout(env);
    let (program, args) = command
        .split_first()
        .ok_or_else(|| DittoError::AuthCommand("command is empty".to_string()))?;

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|err| DittoError::AuthCommand(format!("spawn {program}: {err}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DittoError::AuthCommand("command did not capture stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DittoError::AuthCommand("command did not capture stderr".to_string()))?;

    const MAX_AUTH_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
    let stdout_task = tokio::spawn(read_capped(stdout, MAX_AUTH_COMMAND_OUTPUT_BYTES));
    let stderr_task = tokio::spawn(read_capped(stderr, MAX_AUTH_COMMAND_OUTPUT_BYTES));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            status.map_err(|err| DittoError::AuthCommand(format!("wait {program}: {err}")))?
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DittoError::AuthCommand(format!(
                "command {program} timed out after {}ms",
                timeout.as_millis()
            )));
        }
    };

    let (stdout, stdout_truncated) = stdout_task
        .await
        .map_err(|err| DittoError::AuthCommand(format!("join stdout reader: {err}")))??;
    let (stderr, stderr_truncated) = stderr_task
        .await
        .map_err(|err| DittoError::AuthCommand(format!("join stderr reader: {err}")))??;

    if stdout_truncated {
        return Err(DittoError::AuthCommand(format!(
            "command {program} stdout exceeds {MAX_AUTH_COMMAND_OUTPUT_BYTES} bytes"
        )));
    }
    if stderr_truncated {
        return Err(DittoError::AuthCommand(format!(
            "command {program} stderr exceeds {MAX_AUTH_COMMAND_OUTPUT_BYTES} bytes"
        )));
    }

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(DittoError::AuthCommand(format!(
                "command {program} failed with status {status}"
            )));
        }

        let preview = stderr
            .chars()
            .take(200)
            .collect::<String>()
            .trim()
            .to_string();
        return Err(DittoError::AuthCommand(format!(
            "command {program} failed with status {status}: {preview}"
        )));
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err(DittoError::AuthCommand(format!(
            "command {program} returned empty stdout"
        )));
    }
    Ok(stdout.to_string())
}

fn parse_auth_command_token(stdout: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct AuthCommandOutput {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        access_token: Option<String>,
    }

    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err(DittoError::AuthCommand(
            "command returned empty stdout".to_string(),
        ));
    }

    match serde_json::from_str::<serde_json::Value>(stdout) {
        Ok(serde_json::Value::Object(map)) => {
            let parsed = AuthCommandOutput {
                api_key: map
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                token: map
                    .get("token")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                access_token: map
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
            };
            parsed
                .api_key
                .or(parsed.token)
                .or(parsed.access_token)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    DittoError::AuthCommand("json missing api_key/token/access_token".to_string())
                })
        }
        Ok(serde_json::Value::String(value)) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                return Err(DittoError::AuthCommand(
                    "json string token is empty".to_string(),
                ));
            }
            Ok(value)
        }
        Ok(_) => Err(DittoError::AuthCommand(
            "auth command output must be a json object/string or plain text token".to_string(),
        )),
        Err(_) => Ok(stdout.to_string()),
    }
}

async fn read_capped<R>(mut reader: R, max_bytes: usize) -> Result<(Vec<u8>, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;

    let mut out = Vec::<u8>::new();
    let mut buf = [0u8; 4096];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        if truncated {
            continue;
        }

        let remaining = max_bytes.saturating_sub(out.len());
        if remaining == 0 {
            truncated = true;
            continue;
        }

        if n > remaining {
            out.extend_from_slice(&buf[..remaining]);
            truncated = true;
            continue;
        }

        out.extend_from_slice(&buf[..n]);
    }

    Ok((out, truncated))
}
