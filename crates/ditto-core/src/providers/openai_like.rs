use std::collections::BTreeMap;
use std::time::Duration;

use crate::capabilities::file::{FileContent, FileDeleteResponse, FileObject, FileUploadRequest};
#[cfg(feature = "cap-llm-streaming")]
use futures_util::StreamExt;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::stream::{self, BoxStream};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
#[cfg(all(any(feature = "provider-openai", test), feature = "cap-llm-streaming"))]
use tokio::io::AsyncBufRead;

#[cfg(feature = "provider-openai-compatible")]
use crate::config::resolve_provider_request_auth_optional;
#[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
use crate::config::resolve_provider_request_auth_required;
use crate::config::{Env, HttpAuth, ProviderConfig, RequestAuth};
#[cfg(feature = "cap-llm-streaming")]
use crate::error::DittoError;
use crate::error::Result;
use crate::provider_transport::{apply_http_query_params, resolve_http_provider_config};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const DEFAULT_MAX_BINARY_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[cfg(feature = "cap-llm-streaming")]
fn map_http_kit_sse_error(error: http_kit::Error) -> DittoError {
    let message = error.message();

    if let Some(limit) = message
        .strip_suffix(" must be greater than zero")
        .filter(|limit| !limit.is_empty())
    {
        return crate::invalid_response!(
            "error_detail.sse.limit_must_be_positive",
            "limit" => limit
        );
    }

    if let Some(max_line_bytes) = message.strip_prefix("sse line exceeds max_line_bytes ") {
        return crate::invalid_response!(
            "error_detail.sse.line_too_large",
            "max_line_bytes" => max_line_bytes
        );
    }

    if let Some(max_event_bytes) = message.strip_prefix("sse event exceeds max_event_bytes ") {
        return crate::invalid_response!(
            "error_detail.sse.event_too_large",
            "max_event_bytes" => max_event_bytes
        );
    }

    if let Some(read_error) = message.strip_prefix("read sse line failed: ") {
        return crate::invalid_response!(
            "error_detail.sse.read_line_failed",
            "error" => read_error
        );
    }

    if let Some(decode_error) = message.strip_prefix("invalid sse utf-8: ") {
        return crate::invalid_response!(
            "error_detail.sse.invalid_utf8",
            "error" => decode_error
        );
    }

    DittoError::invalid_response_text(message)
}

#[cfg(feature = "cap-llm-streaming")]
fn adapt_openai_compatible_sse_stream(
    stream: BoxStream<'static, std::result::Result<String, http_kit::Error>>,
) -> BoxStream<'static, Result<String>> {
    Box::pin(stream::unfold(stream, |mut stream| async move {
        match stream.next().await {
            Some(Ok(data)) if data == "[DONE]" => None,
            Some(Ok(data)) => Some((Ok(data), stream)),
            Some(Err(error)) => Some((Err(map_http_kit_sse_error(error)), stream)),
            None => None,
        }
    }))
}

#[cfg(all(any(feature = "provider-openai", test), feature = "cap-llm-streaming"))]
pub(crate) fn openai_compatible_sse_data_stream_from_reader<R>(
    reader: R,
) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    adapt_openai_compatible_sse_stream(http_kit::sse_data_stream_from_reader(reader))
}

#[cfg(feature = "cap-llm-streaming")]
pub(crate) fn openai_compatible_sse_data_stream_from_response(
    response: reqwest::Response,
) -> BoxStream<'static, Result<String>> {
    adapt_openai_compatible_sse_stream(http_kit::sse_data_stream_from_response(response))
}

pub(crate) fn default_http_client() -> reqwest::Client {
    crate::provider_transport::default_http_client(HTTP_TIMEOUT)
}

pub(crate) fn auth_from_api_key(api_key: &str) -> Option<RequestAuth> {
    if api_key.trim().is_empty() {
        return None;
    }
    HttpAuth::bearer(api_key).ok().map(RequestAuth::Http)
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

    #[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
    pub(crate) async fn from_config_required(
        config: &ProviderConfig,
        env: &Env,
        default_keys: &[&str],
    ) -> Result<Self> {
        let auth_header = resolve_provider_request_auth_required(
            config,
            env,
            default_keys,
            "authorization",
            Some("Bearer "),
        )
        .await?;
        let resolved = resolve_http_provider_config(HTTP_TIMEOUT, config, Some(DEFAULT_BASE_URL))?;

        let mut out = Self::new("").with_http_client(resolved.http);
        out.auth = Some(auth_header);
        out.http_query_params = resolved.http_query_params;
        if let Some(base_url) = resolved.base_url {
            out = out.with_base_url(base_url);
        }
        #[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
        if let Some(model) = resolved.default_model {
            out = out.with_model(model);
        }
        Ok(out)
    }

    #[cfg(feature = "provider-openai-compatible")]
    pub(crate) async fn from_config_optional(
        config: &ProviderConfig,
        env: &Env,
        default_keys: &[&str],
    ) -> Result<Self> {
        let auth = resolve_provider_request_auth_optional(
            config,
            env,
            default_keys,
            "authorization",
            Some("Bearer "),
        )
        .await?;
        let resolved = resolve_http_provider_config(HTTP_TIMEOUT, config, Some(DEFAULT_BASE_URL))?;

        let mut out = Self::new("").with_http_client(resolved.http);
        out.auth = auth;
        out.http_query_params = resolved.http_query_params;
        if let Some(base_url) = resolved.base_url {
            out = out.with_base_url(base_url);
        }
        #[cfg(any(feature = "provider-openai", feature = "provider-openai-compatible"))]
        if let Some(model) = resolved.default_model {
            out = out.with_model(model);
        }
        Ok(out)
    }

    pub(crate) fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        apply_auth(req, self.auth.as_ref(), &self.http_query_params)
    }

    pub(crate) fn endpoint(&self, endpoint: &str) -> String {
        http_kit::join_api_base_url_path(&self.base_url, endpoint)
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
            crate::invalid_response!(
                "error_detail.openai_like.file_upload_media_type_invalid",
                "error" => err.to_string()
            )
        })?;
    }

    let form = Form::new()
        .text("purpose", request.purpose)
        .part("file", file_part);

    let mut req = http.post(url);
    req = apply_auth(req, auth, http_query_params);
    let parsed =
        crate::provider_transport::send_checked_json::<FilesUploadResponse>(req.multipart(form))
            .await?;
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
    let parsed = crate::provider_transport::send_checked_json::<FilesListResponse>(req).await?;
    Ok(parsed.data)
}

#[cfg(all(test, feature = "cap-llm-streaming"))]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::StreamExt;
    use futures_util::stream;

    #[tokio::test]
    async fn openai_compatible_sse_stops_at_done_literal() -> Result<()> {
        let sse = concat!("data: hello\n\n", "data: [DONE]\n\n");
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut data_stream =
            openai_compatible_sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        let first = data_stream.next().await.unwrap()?;
        assert_eq!(first, "hello");
        assert!(data_stream.next().await.is_none());
        Ok(())
    }
}

pub(crate) async fn retrieve_file(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> Result<FileObject> {
    let mut req = http.get(url);
    req = apply_auth(req, auth, http_query_params);
    crate::provider_transport::send_checked_json::<FileObject>(req).await
}

pub(crate) async fn delete_file(
    http: &reqwest::Client,
    url: String,
    auth: Option<&RequestAuth>,
    http_query_params: &BTreeMap<String, String>,
) -> Result<FileDeleteResponse> {
    let mut req = http.delete(url);
    req = apply_auth(req, auth, http_query_params);
    crate::provider_transport::send_checked_json::<FileDeleteResponse>(req).await
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
    let response = crate::provider_transport::send_checked(req).await?;

    let headers = response.headers().clone();
    let media_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = crate::provider_transport::read_reqwest_body_bytes_bounded_with_content_length(
        response, &headers, max_bytes,
    )
    .await
    .map_err(|err| {
        crate::invalid_response!(
            "error_detail.openai_like.files_download_response_too_large",
            "max_bytes" => max_bytes.to_string(),
            "error" => err.to_string()
        )
    })?
    .to_vec();

    Ok(FileContent { bytes, media_type })
}
