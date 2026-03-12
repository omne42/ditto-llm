use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::capabilities::file::FileContent;
use crate::foundation::error::Result;
use crate::foundation::error::{DittoError, Result as DittoResult};
use crate::types::{
    VideoContentVariant, VideoDeleteResponse, VideoGenerationError, VideoGenerationRequest,
    VideoGenerationResponse, VideoGenerationStatus, VideoListOrder, VideoListRequest,
    VideoListResponse, VideoRemixRequest, Warning,
};

#[derive(Debug, Deserialize, Default)]
struct VideoObject {
    #[serde(default)]
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    status: VideoGenerationStatus,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created_at: Option<u64>,
    #[serde(default)]
    completed_at: Option<u64>,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    progress: Option<u32>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    remixed_from_video_id: Option<String>,
    #[serde(default)]
    seconds: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    error: Option<VideoGenerationError>,
}

#[derive(Debug, Deserialize, Default)]
struct VideoListObject {
    #[serde(default)]
    data: Vec<Value>,
    #[serde(default)]
    has_more: Option<bool>,
    #[serde(default)]
    last_id: Option<String>,
}

fn videos_url(client: &OpenAiLikeClient) -> String {
    client.endpoint("videos")
}

fn video_url(client: &OpenAiLikeClient, video_id: &str) -> String {
    format!("{}/{}", videos_url(client), video_id.trim())
}

fn video_content_url(client: &OpenAiLikeClient, video_id: &str) -> String {
    format!("{}/content", video_url(client, video_id))
}

fn video_remix_url(client: &OpenAiLikeClient, video_id: &str) -> String {
    format!("{}/remix", video_url(client, video_id))
}

fn merge_provider_options_into_multipart_form(
    mut form: Form,
    options: Option<&Value>,
    reserved_keys: &[&str],
    feature: &str,
    warnings: &mut Vec<Warning>,
) -> Form {
    let Some(options) = options else {
        return form;
    };

    let Some(obj) = options.as_object() else {
        warnings.push(Warning::Unsupported {
            feature: feature.to_string(),
            details: Some("expected provider_options to be a JSON object".to_string()),
        });
        return form;
    };

    for (key, value) in obj {
        if reserved_keys.contains(&key.as_str()) {
            continue;
        }

        match value {
            Value::Null => {}
            Value::String(value) => {
                form = form.text(key.clone(), value.clone());
            }
            Value::Number(value) => {
                form = form.text(key.clone(), value.to_string());
            }
            Value::Bool(value) => {
                form = form.text(key.clone(), if *value { "true" } else { "false" });
            }
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::Null => {}
                        Value::String(value) => {
                            form = form.text(key.clone(), value.clone());
                        }
                        Value::Number(value) => {
                            form = form.text(key.clone(), value.to_string());
                        }
                        Value::Bool(value) => {
                            form = form.text(
                                key.clone(),
                                if *value {
                                    "true".to_string()
                                } else {
                                    "false".to_string()
                                },
                            );
                        }
                        Value::Object(_) | Value::Array(_) => {
                            form = form.text(key.clone(), item.to_string());
                        }
                    }
                }
            }
            Value::Object(_) => {
                form = form.text(key.clone(), value.to_string());
            }
        }
    }

    form
}

fn video_reference_part(upload: crate::types::VideoReferenceUpload) -> DittoResult<Part> {
    let mut part = Part::bytes(upload.data).file_name(upload.filename);
    if let Some(media_type) = upload.media_type.as_deref() {
        part = part.mime_str(media_type).map_err(|err| {
            DittoError::InvalidResponse(format!(
                "invalid video reference media type {media_type:?}: {err}"
            ))
        })?;
    }
    Ok(part)
}

fn parse_video_response(raw: Value, warnings: Vec<Warning>) -> Result<VideoGenerationResponse> {
    let parsed = serde_json::from_value::<VideoObject>(raw.clone())?;
    Ok(VideoGenerationResponse {
        id: parsed.id,
        object: parsed.object,
        status: parsed.status,
        model: parsed.model,
        created_at: parsed.created_at,
        completed_at: parsed.completed_at,
        expires_at: parsed.expires_at,
        progress: parsed.progress,
        prompt: parsed.prompt,
        remixed_from_video_id: parsed.remixed_from_video_id,
        seconds: parsed.seconds,
        size: parsed.size,
        error: parsed.error,
        warnings,
        provider_metadata: Some(raw),
    })
}

pub(super) async fn create(
    provider: &str,
    client: &OpenAiLikeClient,
    default_model: Option<&str>,
    request: VideoGenerationRequest,
) -> Result<VideoGenerationResponse> {
    let VideoGenerationRequest {
        prompt,
        input_reference,
        model,
        seconds,
        size,
        provider_options,
    } = request;

    let selected_provider_options = crate::provider_options::select_provider_options_value(
        provider_options.as_ref(),
        provider,
    )?;
    let selected_model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            default_model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    let mut warnings = Vec::<Warning>::new();
    let req = if let Some(input_reference) = input_reference {
        let mut form = Form::new().text("prompt", prompt.clone());
        if let Some(model) = selected_model.as_ref() {
            form = form.text("model", model.clone());
        }
        if let Some(seconds) = seconds {
            form = form.text("seconds", seconds.to_string());
        }
        if let Some(size) = size.as_deref().filter(|value| !value.trim().is_empty()) {
            form = form.text("size", size.to_string());
        }
        form = form.part("input_reference", video_reference_part(input_reference)?);
        form = merge_provider_options_into_multipart_form(
            form,
            selected_provider_options.as_ref(),
            &["prompt", "input_reference", "model", "seconds", "size"],
            "video.provider_options",
            &mut warnings,
        );
        client
            .apply_auth(client.http.post(videos_url(client)))
            .multipart(form)
    } else {
        let mut body = Map::<String, Value>::new();
        body.insert("prompt".to_string(), Value::String(prompt.clone()));
        if let Some(model) = selected_model.as_ref() {
            body.insert("model".to_string(), Value::String(model.clone()));
        }
        if let Some(seconds) = seconds {
            body.insert("seconds".to_string(), Value::String(seconds.to_string()));
        }
        if let Some(size) = size.as_deref().filter(|value| !value.trim().is_empty()) {
            body.insert("size".to_string(), Value::String(size.to_string()));
        }

        crate::provider_options::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["prompt", "input_reference", "model", "seconds", "size"],
            "video.provider_options",
            &mut warnings,
        );

        client
            .apply_auth(client.http.post(videos_url(client)))
            .json(&body)
    };

    let raw = crate::provider_transport::send_checked_json::<Value>(req).await?;
    let mut response = parse_video_response(raw, warnings)?;
    if response.model.is_none() {
        response.model = selected_model;
    }
    if response.prompt.is_none() {
        response.prompt = Some(prompt);
    }
    if response.seconds.is_none() {
        response.seconds = seconds.map(|value| value.to_string());
    }
    if response.size.is_none() {
        response.size = size;
    }
    Ok(response)
}

pub(super) async fn retrieve(
    client: &OpenAiLikeClient,
    video_id: &str,
) -> Result<VideoGenerationResponse> {
    let raw = crate::provider_transport::send_checked_json::<Value>(
        client.apply_auth(client.http.get(video_url(client, video_id))),
    )
    .await?;
    parse_video_response(raw, Vec::new())
}

pub(super) async fn list(
    client: &OpenAiLikeClient,
    request: VideoListRequest,
) -> Result<VideoListResponse> {
    let VideoListRequest {
        limit,
        after,
        order,
    } = request;
    let mut req = client.apply_auth(client.http.get(videos_url(client)));
    if let Some(limit) = limit {
        req = req.query(&[("limit", limit)]);
    }
    if let Some(after) = after.as_deref().filter(|value| !value.trim().is_empty()) {
        req = req.query(&[("after", after)]);
    }
    if let Some(order) = order {
        let value = match order {
            VideoListOrder::Asc => "asc",
            VideoListOrder::Desc => "desc",
        };
        req = req.query(&[("order", value)]);
    }

    let raw = crate::provider_transport::send_checked_json::<Value>(req).await?;
    let parsed = serde_json::from_value::<VideoListObject>(raw.clone())?;
    let mut videos = Vec::<VideoGenerationResponse>::with_capacity(parsed.data.len());
    for item in parsed.data {
        videos.push(parse_video_response(item, Vec::new())?);
    }

    Ok(VideoListResponse {
        videos,
        after: parsed.last_id,
        has_more: parsed.has_more,
        warnings: Vec::new(),
        provider_metadata: Some(raw),
    })
}

pub(super) async fn delete(
    client: &OpenAiLikeClient,
    video_id: &str,
) -> Result<VideoDeleteResponse> {
    crate::provider_transport::send_checked_json::<VideoDeleteResponse>(
        client.apply_auth(client.http.delete(video_url(client, video_id))),
    )
    .await
}

pub(super) async fn download_content(
    client: &OpenAiLikeClient,
    video_id: &str,
    variant: Option<VideoContentVariant>,
) -> Result<FileContent> {
    let mut req = client
        .apply_auth(client.http.get(video_content_url(client, video_id)))
        .header(ACCEPT, "application/binary");
    if let Some(variant) = variant {
        let value = match variant {
            VideoContentVariant::Video => "video",
            VideoContentVariant::Thumbnail => "thumbnail",
            VideoContentVariant::Spritesheet => "spritesheet",
        };
        req = req.query(&[("variant", value)]);
    }

    let response = crate::provider_transport::send_checked(req).await?;
    let headers = response.headers().clone();
    let media_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = crate::provider_transport::read_reqwest_body_bytes_bounded_with_content_length(
        response,
        &headers,
        client.max_binary_response_bytes,
    )
    .await?;

    Ok(FileContent {
        bytes: bytes.to_vec(),
        media_type,
    })
}

pub(super) async fn remix(
    provider: &str,
    client: &OpenAiLikeClient,
    video_id: &str,
    request: VideoRemixRequest,
) -> Result<VideoGenerationResponse> {
    let selected_provider_options = crate::provider_options::select_provider_options_value(
        request.provider_options.as_ref(),
        provider,
    )?;
    let mut warnings = Vec::<Warning>::new();
    let mut body = Map::<String, Value>::new();
    body.insert("prompt".to_string(), Value::String(request.prompt));
    crate::provider_options::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &["prompt"],
        "video.remix.provider_options",
        &mut warnings,
    );

    let raw = crate::provider_transport::send_checked_json::<Value>(
        client
            .apply_auth(client.http.post(video_remix_url(client, video_id)))
            .json(&body),
    )
    .await?;
    parse_video_response(raw, warnings)
}
