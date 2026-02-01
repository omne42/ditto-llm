use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::types::{ImageGenerationRequest, ImageGenerationResponse, ImageSource, Usage, Warning};
use crate::{DittoError, Result};

#[derive(Debug, Deserialize)]
struct ImagesGenerationResponse {
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    data: Vec<ImageGenerationData>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ImageGenerationData {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
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

    let selected_provider_options =
        crate::types::select_provider_options_value(provider_options.as_ref(), provider)?;
    let mut warnings = Vec::<Warning>::new();

    let mut body = Map::<String, Value>::new();
    body.insert("model".to_string(), Value::String(model.clone()));
    body.insert("prompt".to_string(), Value::String(prompt));
    if let Some(n) = n {
        body.insert("n".to_string(), Value::Number(n.into()));
    }
    if let Some(size) = size.as_deref().filter(|s| !s.trim().is_empty()) {
        body.insert("size".to_string(), Value::String(size.to_string()));
    }
    if let Some(format) = response_format {
        body.insert("response_format".to_string(), serde_json::to_value(format)?);
    }

    crate::types::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &["model", "prompt", "n", "size", "response_format"],
        "image.provider_options",
        &mut warnings,
    );

    let url = client.endpoint("images/generations");
    let response = client
        .apply_auth(client.http.post(url))
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(DittoError::Api { status, body: text });
    }

    let parsed = response.json::<ImagesGenerationResponse>().await?;
    let usage = parsed.usage.as_ref().map(parse_usage).unwrap_or_default();

    let mut images = Vec::<ImageSource>::new();
    let mut revised_prompts = Vec::<String>::new();
    for item in parsed.data {
        if let Some(prompt) = item
            .revised_prompt
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            revised_prompts.push(prompt.to_string());
        }

        if let Some(url) = item.url.as_deref().filter(|v| !v.trim().is_empty()) {
            images.push(ImageSource::Url {
                url: url.to_string(),
            });
            continue;
        }
        if let Some(data) = item.b64_json.as_deref().filter(|v| !v.trim().is_empty()) {
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

    let mut provider_metadata = serde_json::json!({ "model": model });
    if let Some(created) = parsed.created {
        provider_metadata
            .as_object_mut()
            .expect("provider_metadata is object")
            .insert("created".to_string(), Value::Number(created.into()));
    }
    if !revised_prompts.is_empty() {
        provider_metadata
            .as_object_mut()
            .expect("provider_metadata is object")
            .insert(
                "revised_prompts".to_string(),
                Value::Array(revised_prompts.into_iter().map(Value::String).collect()),
            );
    }

    Ok(ImageGenerationResponse {
        images,
        usage,
        warnings,
        provider_metadata: Some(provider_metadata),
    })
}
