use std::collections::BTreeMap;
use std::time::Duration;

use crate::file::{FileContent, FileDeleteResponse, FileObject, FileUploadRequest};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::{DittoError, Result};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const DEFAULT_MAX_BINARY_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn join_endpoint(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let endpoint = endpoint.trim_start_matches('/');
    if base.ends_with(&format!("/{endpoint}")) {
        base.to_string()
    } else {
        format!("{base}/{endpoint}")
    }
}

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

#[derive(Clone)]
pub(crate) struct OpenAiLikeClient {
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) auth: Option<RequestAuth>,
    pub(crate) model: String,
    pub(crate) http_query_params: BTreeMap<String, String>,
    pub(crate) max_binary_response_bytes: usize,
}

impl OpenAiLikeClient {
    pub(crate) fn new(api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        let http = default_http_client();
        let auth = auth_from_api_key(&api_key);

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            model: String::new(),
            http_query_params: BTreeMap::new(),
            max_binary_response_bytes: DEFAULT_MAX_BINARY_RESPONSE_BYTES,
        }
    }

    pub(crate) fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub(crate) fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub(crate) fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub(crate) fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.max_binary_response_bytes = max_bytes.max(1);
        self
    }

    pub(crate) async fn from_config_required(
        config: &ProviderConfig,
        env: &Env,
        default_keys: &[&str],
    ) -> Result<Self> {
        let auth_header = resolve_auth_required(config, env, default_keys).await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                HTTP_TIMEOUT,
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    pub(crate) async fn from_config_optional(
        config: &ProviderConfig,
        env: &Env,
        default_keys: &[&str],
    ) -> Result<Self> {
        let auth = resolve_auth_optional(config, env, default_keys).await?;

        let mut out = Self::new("");
        out.auth = auth;
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                HTTP_TIMEOUT,
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            out = out.with_model(model);
        }
        Ok(out)
    }

    pub(crate) fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        apply_auth(req, self.auth.as_ref(), &self.http_query_params)
    }

    pub(crate) fn endpoint(&self, endpoint: &str) -> String {
        join_endpoint(&self.base_url, endpoint)
    }

    pub(crate) async fn upload_file_with_purpose(
        &self,
        request: FileUploadRequest,
    ) -> Result<String> {
        self::upload_file_with_purpose(
            &self.http,
            self.endpoint("files"),
            self.auth.as_ref(),
            &self.http_query_params,
            request,
        )
        .await
    }

    pub(crate) async fn list_files(&self) -> Result<Vec<FileObject>> {
        self::list_files(
            &self.http,
            self.endpoint("files"),
            self.auth.as_ref(),
            &self.http_query_params,
        )
        .await
    }

    pub(crate) async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        let url = format!("{}/{}", self.endpoint("files"), file_id.trim());
        self::retrieve_file(&self.http, url, self.auth.as_ref(), &self.http_query_params).await
    }

    pub(crate) async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        let url = format!("{}/{}", self.endpoint("files"), file_id.trim());
        self::delete_file(&self.http, url, self.auth.as_ref(), &self.http_query_params).await
    }

    pub(crate) async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        let url = format!("{}/{}/content", self.endpoint("files"), file_id.trim());
        self::download_file_content(
            &self.http,
            url,
            self.auth.as_ref(),
            &self.http_query_params,
            self.max_binary_response_bytes,
        )
        .await
    }
}

pub(crate) async fn upload_file_with_purpose(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
    request: FileUploadRequest,
) -> Result<String> {
    #[derive(Deserialize)]
    struct FilesUploadResponse {
        id: String,
    }

    let mut file_part = Part::bytes(request.bytes).file_name(request.filename);
    if let Some(media_type) = request.media_type.as_deref() {
        file_part = file_part.mime_str(media_type).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid file upload media type: {err}"))
        })?;
    }

    let form = Form::new()
        .text("purpose", request.purpose)
        .part("file", file_part);

    let mut req = http.post(url);
    req = apply_auth(req, auth, http_query_params);
    let parsed =
        crate::utils::http::send_checked_json::<FilesUploadResponse>(req.multipart(form)).await?;
    Ok(parsed.id)
}

pub(crate) async fn list_files(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> Result<Vec<FileObject>> {
    #[derive(Deserialize)]
    struct FilesListResponse {
        data: Vec<FileObject>,
    }

    let mut req = http.get(url);
    req = apply_auth(req, auth, http_query_params);
    let parsed = crate::utils::http::send_checked_json::<FilesListResponse>(req).await?;
    Ok(parsed.data)
}

pub(crate) async fn retrieve_file(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> Result<FileObject> {
    let mut req = http.get(url);
    req = apply_auth(req, auth, http_query_params);
    crate::utils::http::send_checked_json::<FileObject>(req).await
}

pub(crate) async fn delete_file(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> Result<FileDeleteResponse> {
    let mut req = http.delete(url);
    req = apply_auth(req, auth, http_query_params);
    crate::utils::http::send_checked_json::<FileDeleteResponse>(req).await
}

pub(crate) async fn download_file_content(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<FileContent> {
    let mut req = http.get(url);
    req = apply_auth(req, auth, http_query_params);
    let response = crate::utils::http::send_checked(req).await?;

    let headers = response.headers().clone();
    let media_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = crate::utils::http::read_reqwest_body_bytes_bounded_with_content_length(
        response, &headers, max_bytes,
    )
    .await
    .map_err(|err| {
        DittoError::InvalidResponse(format!(
            "files download response too large (max={max_bytes}): {err}"
        ))
    })?
    .to_vec();

    Ok(FileContent { bytes, media_type })
}
