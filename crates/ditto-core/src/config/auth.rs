#![allow(dead_code)]

use std::fmt::Display;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;

use crate::error::{DittoError, Result};
use ::secret_kit::{
    SecretResolutionContext, SecretResolver, looks_like_secret_spec,
    resolve_string_if_secret_with_runtime,
};

use super::env::Env;
use super::provider_config::{ProviderAuth, ProviderConfig};

fn invalid_auth_header_name() -> DittoError {
    crate::invalid_response!("error_detail.auth.header_name_empty")
}

fn invalid_auth_header_name_with_error(header: &str, error: impl Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.auth.header_name_invalid",
        "header" => header,
        "error" => error.to_string()
    )
}

fn invalid_auth_header_value(header: &str, error: impl Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.auth.header_value_invalid",
        "header" => header,
        "error" => error.to_string()
    )
}

fn invalid_auth_query_param_name() -> DittoError {
    crate::invalid_response!("error_detail.auth.query_param_name_empty")
}

fn static_request_auth_unsupported(auth_kind: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.auth.static_request_auth_unsupported",
        "auth_kind" => auth_kind
    )
}

fn token_string_auth_unsupported(auth_kind: &str) -> DittoError {
    crate::invalid_response!(
        "error_detail.auth.token_string_unsupported",
        "auth_kind" => auth_kind
    )
}

fn missing_api_key_env(keys: &str) -> DittoError {
    crate::auth_command_error!("error_detail.auth.missing_api_key_env", "keys" => keys)
}

fn command_empty() -> DittoError {
    crate::auth_command_error!("error_detail.auth.command_empty")
}

fn command_spawn_failed(program: &str, error: impl Display) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_spawn_failed",
        "program" => program,
        "error" => error.to_string()
    )
}

fn command_stdout_not_captured(program: &str) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_stdout_not_captured",
        "program" => program
    )
}

fn command_stderr_not_captured(program: &str) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_stderr_not_captured",
        "program" => program
    )
}

fn command_wait_failed(program: &str, error: impl Display) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_wait_failed",
        "program" => program,
        "error" => error.to_string()
    )
}

fn command_timeout(program: &str, timeout_ms: u128) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_timeout",
        "program" => program,
        "timeout_ms" => timeout_ms.to_string()
    )
}

fn command_reader_join_failed(stream: &str, error: impl Display) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_reader_join_failed",
        "stream" => stream,
        "error" => error.to_string()
    )
}

fn command_stdout_too_large(program: &str, max_bytes: usize) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_stdout_too_large",
        "program" => program,
        "max_bytes" => max_bytes.to_string()
    )
}

fn command_stderr_too_large(program: &str, max_bytes: usize) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_stderr_too_large",
        "program" => program,
        "max_bytes" => max_bytes.to_string()
    )
}

fn command_failed_status(program: &str, status: &str) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_failed_status",
        "program" => program,
        "status" => status
    )
}

fn command_empty_stdout(program: &str) -> DittoError {
    crate::auth_command_error!(
        "error_detail.auth.command_empty_stdout",
        "program" => program
    )
}

fn auth_json_missing_token_fields() -> DittoError {
    crate::auth_command_error!("error_detail.auth.json_missing_token_fields")
}

fn auth_json_string_token_empty() -> DittoError {
    crate::auth_command_error!("error_detail.auth.json_string_token_empty")
}

fn auth_output_shape_invalid() -> DittoError {
    crate::auth_command_error!("error_detail.auth.output_shape_invalid")
}

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
            return Err(invalid_auth_header_name());
        }

        let header = HeaderName::from_bytes(header.as_bytes())
            .map_err(|err| invalid_auth_header_name_with_error(header, err))?;

        let mut out = String::new();
        if let Some(prefix) = prefix {
            out.push_str(prefix);
        }
        out.push_str(token);
        let mut value = HeaderValue::from_str(&out)
            .map_err(|err| invalid_auth_header_value(header.as_str(), err))?;
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
            return Err(invalid_auth_query_param_name());
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
            Err(static_request_auth_unsupported("sigv4/oauth"))
        }
    }
}

fn default_api_key_env_auth() -> ProviderAuth {
    ProviderAuth::ApiKeyEnv { keys: Vec::new() }
}

#[cfg(any(
    feature = "provider-anthropic",
    feature = "provider-cohere",
    feature = "provider-google",
    feature = "provider-openai",
    feature = "provider-openai-compatible"
))]
pub(crate) async fn resolve_provider_request_auth_required(
    config: &ProviderConfig,
    env: &Env,
    default_keys: &[&str],
    default_header: &str,
    default_prefix: Option<&str>,
) -> Result<RequestAuth> {
    let auth = config.auth.clone().unwrap_or_else(default_api_key_env_auth);
    resolve_request_auth_with_default_keys(&auth, env, default_keys, default_header, default_prefix)
        .await
}

pub(crate) async fn resolve_provider_request_auth_optional(
    config: &ProviderConfig,
    env: &Env,
    default_keys: &[&str],
    default_header: &str,
    default_prefix: Option<&str>,
) -> Result<Option<RequestAuth>> {
    match config.auth.as_ref() {
        Some(auth) => Ok(Some(
            resolve_request_auth_with_default_keys(
                auth,
                env,
                default_keys,
                default_header,
                default_prefix,
            )
            .await?,
        )),
        None => {
            let has_default_key = default_keys.iter().any(|key| env.get(key).is_some());
            if !has_default_key {
                return Ok(None);
            }

            let auth = default_api_key_env_auth();
            Ok(Some(
                resolve_request_auth_with_default_keys(
                    &auth,
                    env,
                    default_keys,
                    default_header,
                    default_prefix,
                )
                .await?,
            ))
        }
    }
}

pub async fn resolve_auth_token(auth: &ProviderAuth, env: &Env) -> Result<String> {
    const DEFAULT_KEYS: &[&str] = &["OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];
    resolve_auth_token_with_default_keys(auth, env, DEFAULT_KEYS).await
}

pub async fn resolve_auth_token_with_default_keys_and_resolver<R>(
    auth: &ProviderAuth,
    env: &Env,
    default_keys: &[&str],
    resolver: &R,
) -> Result<String>
where
    R: SecretResolver + ?Sized,
{
    match auth {
        ProviderAuth::ApiKeyEnv { keys }
        | ProviderAuth::HttpHeaderEnv { keys, .. }
        | ProviderAuth::QueryParamEnv { keys, .. } => {
            if keys.is_empty() {
                for key in default_keys {
                    if let Some(value) = env.get(key) {
                        return resolve_secret_if_needed_with_resolver(value, env, resolver).await;
                    }
                }
                return Err(missing_api_key_env(&default_keys.join(", ")));
            }
            for key in keys {
                if let Some(value) = env.get(key.as_str()) {
                    return resolve_secret_if_needed_with_resolver(value, env, resolver).await;
                }
            }
            Err(missing_api_key_env(&keys.join(", ")))
        }
        ProviderAuth::Command { command }
        | ProviderAuth::HttpHeaderCommand { command, .. }
        | ProviderAuth::QueryParamCommand { command, .. } => {
            let stdout = run_auth_command(command, env).await?;
            let token = parse_auth_command_token(stdout.as_str())?;
            resolve_secret_if_needed_with_resolver(token, env, resolver).await
        }
        ProviderAuth::SigV4 { .. } | ProviderAuth::OAuthClientCredentials { .. } => {
            Err(token_string_auth_unsupported("sigv4/oauth"))
        }
    }
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
                        return resolve_string_if_secret_with_runtime(&value, env, env)
                            .await
                            .map_err(Into::into);
                    }
                }
                return Err(missing_api_key_env(&default_keys.join(", ")));
            }
            for key in keys {
                if let Some(value) = env.get(key.as_str()) {
                    return resolve_string_if_secret_with_runtime(&value, env, env)
                        .await
                        .map_err(Into::into);
                }
            }
            Err(missing_api_key_env(&keys.join(", ")))
        }
        ProviderAuth::Command { command }
        | ProviderAuth::HttpHeaderCommand { command, .. }
        | ProviderAuth::QueryParamCommand { command, .. } => {
            let stdout = run_auth_command(command, env).await?;
            let token = parse_auth_command_token(stdout.as_str())?;
            resolve_string_if_secret_with_runtime(&token, env, env)
                .await
                .map_err(Into::into)
        }
        ProviderAuth::SigV4 { .. } | ProviderAuth::OAuthClientCredentials { .. } => {
            Err(token_string_auth_unsupported("sigv4/oauth"))
        }
    }
}

async fn resolve_secret_if_needed_with_resolver<R>(
    raw: String,
    env: &Env,
    resolver: &R,
) -> Result<String>
where
    R: SecretResolver + ?Sized,
{
    let raw = raw.trim().to_string();
    if looks_like_secret_spec(&raw) {
        return resolver
            .resolve_secret(raw.as_str(), SecretResolutionContext::new(env, env))
            .await
            .map(|secret| secret.into_owned())
            .map_err(Into::into);
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
    let (program, args) = command.split_first().ok_or_else(command_empty)?;

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|err| command_spawn_failed(program, err))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| command_stdout_not_captured(program))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| command_stderr_not_captured(program))?;

    const MAX_AUTH_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
    let stdout_task = tokio::spawn(read_capped(stdout, MAX_AUTH_COMMAND_OUTPUT_BYTES));
    let stderr_task = tokio::spawn(read_capped(stderr, MAX_AUTH_COMMAND_OUTPUT_BYTES));

    let timeout_error = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            let status = status.map_err(|err| command_wait_failed(program, err))?;
            Ok(status)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(command_timeout(program, timeout.as_millis()))
        }
    };

    let status = match timeout_error {
        Ok(status) => status,
        Err(err) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(err);
        }
    };

    let (stdout, stdout_truncated) = stdout_task
        .await
        .map_err(|err| command_reader_join_failed("stdout", err))??;
    let (_stderr, stderr_truncated) = stderr_task
        .await
        .map_err(|err| command_reader_join_failed("stderr", err))??;

    if stdout_truncated {
        return Err(command_stdout_too_large(
            program,
            MAX_AUTH_COMMAND_OUTPUT_BYTES,
        ));
    }
    if stderr_truncated {
        return Err(command_stderr_too_large(
            program,
            MAX_AUTH_COMMAND_OUTPUT_BYTES,
        ));
    }

    if !status.success() {
        return Err(command_failed_status(program, &status.to_string()));
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err(command_empty_stdout(program));
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
        return Err(command_empty_stdout("auth command"));
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
                .ok_or_else(auth_json_missing_token_fields)
        }
        Ok(serde_json::Value::String(value)) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                return Err(auth_json_string_token_empty());
            }
            Ok(value)
        }
        Ok(_) => Err(auth_output_shape_invalid()),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[tokio::test]
    async fn resolves_auth_token_with_custom_default_keys() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
        };
        let auth = ProviderAuth::ApiKeyEnv { keys: Vec::new() };
        let token = resolve_auth_token_with_default_keys(&auth, &env, &["DITTO_TEST_KEY"]).await?;
        assert_eq!(token, "sk-test");
        Ok(())
    }

    #[tokio::test]
    async fn resolves_auth_token_from_secret_spec_in_env_value() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([
                (
                    "DITTO_TEST_KEY".to_string(),
                    "secret://env/REAL_TEST_KEY".to_string(),
                ),
                ("REAL_TEST_KEY".to_string(), "sk-test".to_string()),
            ]),
        };
        let auth = ProviderAuth::ApiKeyEnv {
            keys: vec!["DITTO_TEST_KEY".to_string()],
        };
        let token = resolve_auth_token_with_default_keys(&auth, &env, &["DITTO_TEST_KEY"]).await?;
        assert_eq!(token, "sk-test");
        Ok(())
    }

    #[tokio::test]
    async fn resolves_http_header_env_auth() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
        };
        let auth = ProviderAuth::HttpHeaderEnv {
            header: "api-key".to_string(),
            keys: vec!["DITTO_TEST_KEY".to_string()],
            prefix: None,
        };
        let resolved = resolve_request_auth_with_default_keys(
            &auth,
            &env,
            &["DITTO_TEST_KEY"],
            "authorization",
            Some("Bearer "),
        )
        .await?;
        let RequestAuth::Http(resolved) = resolved else {
            panic!("expected http header auth");
        };
        assert_eq!(resolved.header.as_str(), "api-key");
        assert_eq!(resolved.value.to_str().unwrap_or_default(), "sk-test");
        Ok(())
    }

    #[tokio::test]
    async fn resolves_query_param_env_auth() -> Result<()> {
        let env = Env {
            dotenv: BTreeMap::from([("DITTO_TEST_KEY".to_string(), "sk-test".to_string())]),
        };
        let auth = ProviderAuth::QueryParamEnv {
            param: "api_key".to_string(),
            keys: vec!["DITTO_TEST_KEY".to_string()],
            prefix: None,
        };
        let resolved = resolve_request_auth_with_default_keys(
            &auth,
            &env,
            &["DITTO_TEST_KEY"],
            "authorization",
            Some("Bearer "),
        )
        .await?;
        let RequestAuth::QueryParam(resolved) = resolved else {
            panic!("expected query param auth");
        };
        assert_eq!(resolved.param, "api_key");
        assert_eq!(resolved.value, "sk-test");
        Ok(())
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn auth_command_runner_discards_stderr_from_errors() -> Result<()> {
        let env = Env::default();
        let err = run_auth_command(
            &[
                "sh".to_string(),
                "-c".to_string(),
                "echo leaked-secret >&2; exit 1".to_string(),
            ],
            &env,
        )
        .await
        .unwrap_err();

        let rendered = err.to_string();
        let DittoError::AuthCommand(message) = &err else {
            panic!("expected auth command error");
        };
        let catalog = message
            .as_catalog()
            .expect("auth command should be catalog-backed");
        assert_eq!(catalog.code(), "error_detail.auth.command_failed_status");
        assert_eq!(catalog.arg("stderr"), None);
        assert!(!rendered.contains("leaked-secret"));
        assert!(!rendered.contains("stderr="));
        Ok(())
    }
}
