use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::{DittoError, Result};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(crate) fn auth_from_api_key(api_key: &str) -> Option<RequestAuth> {
    if api_key.trim().is_empty() {
        return None;
    }
    HttpAuth::bearer(api_key).ok().map(RequestAuth::Http)
}

pub(crate) async fn resolve_auth_required(
    config: &ProviderConfig,
    env: &Env,
    default_keys: &[&str],
) -> Result<RequestAuth> {
    let auth = config
        .auth
        .clone()
        .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
    resolve_request_auth_with_default_keys(
        &auth,
        env,
        default_keys,
        "authorization",
        Some("Bearer "),
    )
    .await
}

pub(crate) async fn resolve_auth_optional(
    config: &ProviderConfig,
    env: &Env,
    default_keys: &[&str],
) -> Result<Option<RequestAuth>> {
    match config.auth.clone() {
        Some(auth) => Ok(Some(
            resolve_request_auth_with_default_keys(
                &auth,
                env,
                default_keys,
                "authorization",
                Some("Bearer "),
            )
            .await?,
        )),
        None => Ok(default_keys
            .iter()
            .find_map(|key| env.get(key))
            .and_then(|token| HttpAuth::bearer(&token).ok().map(RequestAuth::Http))),
    }
}

pub(crate) fn apply_auth(
    req: reqwest::RequestBuilder,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> reqwest::RequestBuilder {
    let req = match auth {
        Some(auth) => auth.apply(req),
        None => req,
    };
    apply_http_query_params(req, http_query_params)
}

pub(crate) async fn upload_file_with_purpose(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
    filename: impl Into<String>,
    bytes: Vec<u8>,
    purpose: impl Into<String>,
    media_type: Option<&str>,
) -> Result<String> {
    #[derive(Deserialize)]
    struct FilesUploadResponse {
        id: String,
    }

    let filename = filename.into();
    let mut file_part = Part::bytes(bytes).file_name(filename);
    if let Some(media_type) = media_type {
        file_part = file_part.mime_str(media_type).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid file upload media type: {err}"))
        })?;
    }

    let form = Form::new()
        .text("purpose", purpose.into())
        .part("file", file_part);

    let mut req = http.post(url);
    req = apply_auth(req, auth, http_query_params);
    let response = req.multipart(form).send().await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let parsed = response.json::<FilesUploadResponse>().await?;
    Ok(parsed.id)
}
