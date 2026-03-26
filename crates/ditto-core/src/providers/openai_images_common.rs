use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::contracts::{ImageSource, Usage, Warning};
use crate::error::Result;
use crate::types::{
    ImageEditRequest, ImageEditResponse, ImageEditUpload, ImageGenerationRequest,
    ImageGenerationResponse, ImageResponseFormat,
};

#[derive(Debug, Deserialize)]
struct ImagesResponse {
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    data: Vec<ImageResponseData>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ImageResponseData {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
}

fn image_response_format_to_str(format: ImageResponseFormat) -> &'static str {
    match format {
        ImageResponseFormat::Url => "url",
        ImageResponseFormat::Base64Json => "b64_json",
    }
}

fn parse_usage(value: &Value) -> Usage {
    let mut usage = Usage::default();
    if let Some(obj) = value.as_object() {
        usage.input_tokens = obj.get("prompt_tokens").and_then(Value::as_u64);
        usage.output_tokens = obj.get("completion_tokens").and_then(Value::as_u64);
        usage.total_tokens = obj.get("total_tokens").and_then(Value::as_u64);
    }
    usage.merge_total();
    usage
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

fn parse_images_response(
    model: String,
    parsed: ImagesResponse,
    mut warnings: Vec<Warning>,
) -> ImageGenerationResponse {
    let usage = parsed.usage.as_ref().map(parse_usage).unwrap_or_default();

    let mut images = Vec::<ImageSource>::new();
    let mut revised_prompts = Vec::<String>::new();
    for item in parsed.data {
        if let Some(prompt) = item
            .revised_prompt
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            revised_prompts.push(prompt.to_string());
        }

        if let Some(url) = item.url.as_deref().filter(|value| !value.trim().is_empty()) {
            images.push(ImageSource::Url {
                url: url.to_string(),
            });
            continue;
        }
        if let Some(data) = item
            .b64_json
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            images.push(ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: data.to_string(),
            });
            continue;
        }

        warnings.push(Warning::Compatibility {
            feature: "image.data".to_string(),
            details: "image item is missing both url and b64_json".to_string(),
        });
    }

    let mut provider_metadata = Map::<String, Value>::new();
    provider_metadata.insert("model".to_string(), Value::String(model));
    if let Some(created) = parsed.created {
        provider_metadata.insert("created".to_string(), Value::Number(created.into()));
    }
    if !revised_prompts.is_empty() {
        provider_metadata.insert(
            "revised_prompts".to_string(),
            Value::Array(revised_prompts.into_iter().map(Value::String).collect()),
        );
    }

    ImageGenerationResponse {
        images,
        usage,
        warnings,
        provider_metadata: Some(Value::Object(provider_metadata)),
    }
}

fn image_part(upload: ImageEditUpload) -> Result<Part> {
    let mut part = Part::bytes(upload.data).file_name(upload.filename);
    if let Some(media_type) = upload.media_type.as_deref() {
        part = part.mime_str(media_type).map_err(|err| {
            crate::invalid_response!(
                "error_detail.openai.image_edit_media_type_invalid",
                "media_type" => format!("{media_type:?}"),
                "error" => err.to_string()
            )
        })?;
    }
    Ok(part)
}

pub(super) async fn generate_images(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: ImageGenerationRequest,
) -> Result<ImageGenerationResponse> {
    let ImageGenerationRequest {
        prompt,
        model: _,
        n,
        size,
        response_format,
        provider_options,
    } = request;

    let selected_provider_options = crate::provider_options::select_provider_options_value(
        provider_options.as_ref(),
        provider,
    )?;
    let mut warnings = Vec::<Warning>::new();

    let mut body = Map::<String, Value>::new();
    body.insert("model".to_string(), Value::String(model.clone()));
    body.insert("prompt".to_string(), Value::String(prompt));
    if let Some(n) = n {
        body.insert("n".to_string(), Value::Number(n.into()));
    }
    if let Some(size) = size.as_deref().filter(|value| !value.trim().is_empty()) {
        body.insert("size".to_string(), Value::String(size.to_string()));
    }
    if let Some(format) = response_format {
        body.insert("response_format".to_string(), serde_json::to_value(format)?);
    }

    crate::provider_options::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &["model", "prompt", "n", "size", "response_format"],
        "image.provider_options",
        &mut warnings,
    );

    let url = client.endpoint("images/generations");
    let parsed = crate::provider_transport::send_checked_json::<ImagesResponse>(
        client.apply_auth(client.http.post(url)).json(&body),
    )
    .await?;

    Ok(parse_images_response(model, parsed, warnings))
}

pub(super) async fn edit_images(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: ImageEditRequest,
) -> Result<ImageEditResponse> {
    let ImageEditRequest {
        prompt,
        images,
        mask,
        model: _,
        n,
        size,
        response_format,
        provider_options,
    } = request;

    if images.is_empty() {
        return Err(crate::invalid_response!(
            "error_detail.openai.image_edit_requires_input_image"
        ));
    }

    let selected_provider_options = crate::provider_options::select_provider_options_value(
        provider_options.as_ref(),
        provider,
    )?;
    let mut warnings = Vec::<Warning>::new();
    let mut form = Form::new()
        .text("model", model.clone())
        .text("prompt", prompt);
    if let Some(n) = n {
        form = form.text("n", n.to_string());
    }
    if let Some(size) = size.as_deref().filter(|value| !value.trim().is_empty()) {
        form = form.text("size", size.to_string());
    }
    if let Some(format) = response_format {
        form = form.text("response_format", image_response_format_to_str(format));
    }
    for image in images {
        form = form.part("image", image_part(image)?);
    }
    if let Some(mask) = mask {
        form = form.part("mask", image_part(mask)?);
    }
    form = merge_provider_options_into_multipart_form(
        form,
        selected_provider_options.as_ref(),
        &[
            "model",
            "prompt",
            "image",
            "mask",
            "n",
            "size",
            "response_format",
        ],
        "image_edit.provider_options",
        &mut warnings,
    );

    let url = client.endpoint("images/edits");
    let parsed = crate::provider_transport::send_checked_json::<ImagesResponse>(
        client.apply_auth(client.http.post(url)).multipart(form),
    )
    .await?;

    Ok(parse_images_response(model, parsed, warnings))
}
