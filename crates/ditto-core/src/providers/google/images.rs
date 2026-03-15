#[cfg(any(feature = "cap-image-generation", feature = "cap-image-edit"))]
mod google_images_impl {
    use async_trait::async_trait;
    use serde::Deserialize;
    use serde_json::{Map, Number, Value};

    use super::Google;
    use crate::capabilities::ImageGenerationModel;
    use crate::config::{Env, ProviderConfig};
    use crate::contracts::{ImageSource, Warning};
    use crate::provider_options::select_provider_options_value;
    use crate::types::{
        ImageGenerationRequest, ImageGenerationResponse, ImageResponseFormat,
    };
    use crate::error::{DittoError, Result};

    #[derive(Clone)]
    pub struct GoogleImages {
        client: Google,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct GoogleImageProviderOptions {
        aspect_ratio: Option<String>,
        guidance_scale: Option<f32>,
        safety_filter_level: Option<String>,
        person_generation: Option<String>,
        include_safety_attributes: Option<bool>,
        include_rai_reason: Option<bool>,
        language: Option<String>,
        output_mime_type: Option<String>,
        output_compression_quality: Option<u32>,
        image_size: Option<String>,
        number_of_images: Option<u32>,
    }

    #[derive(Debug, Default, Deserialize)]
    struct GoogleImagePredictResponse {
        #[serde(default)]
        predictions: Vec<GoogleGeneratedImage>,
        #[serde(default, rename = "positivePromptSafetyAttributes")]
        positive_prompt_safety_attributes: Option<Value>,
    }

    #[derive(Debug, Default, Deserialize)]
    struct GoogleGeneratedImage {
        #[serde(default, rename = "bytesBase64Encoded")]
        bytes_base64_encoded: Option<String>,
        #[serde(default, rename = "mimeType")]
        mime_type: Option<String>,
        #[serde(default, rename = "gcsUri")]
        gcs_uri: Option<String>,
        #[serde(default, rename = "raiFilteredReason")]
        rai_filtered_reason: Option<String>,
        #[serde(default, rename = "enhancedPrompt")]
        enhanced_prompt: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
    }

    impl GoogleImages {
        pub fn new(api_key: impl Into<String>) -> Self {
            Self {
                client: Google::new(api_key),
            }
        }

        pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
            self.client = self.client.with_http_client(http);
            self
        }

        pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
            self.client = self.client.with_base_url(base_url);
            self
        }

        pub fn with_model(mut self, model: impl Into<String>) -> Self {
            self.client = self.client.with_model(model);
            self
        }

        pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
            Ok(Self {
                client: Google::from_config(config, env).await?,
            })
        }

        fn resolve_model<'a>(&'a self, request: &'a ImageGenerationRequest) -> Result<&'a str> {
            if let Some(model) = request
                .model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
            {
                return Ok(model);
            }
            if !self.client.default_model.trim().is_empty() {
                return Ok(self.client.default_model.as_str());
            }
            Err(DittoError::provider_model_missing(
                "google image",
                "set request.model or GoogleImages::with_model",
            ))
        }

        fn predict_url(&self, model: &str) -> String {
            let base = self.client.base_url.trim_end_matches('/');
            let path = Google::model_path(model);
            format!("{base}/{path}:predict")
        }
    }

    fn parse_google_image_provider_options(
        value: Option<Value>,
        warnings: &mut Vec<Warning>,
    ) -> Result<GoogleImageProviderOptions> {
        let Some(value) = value else {
            return Ok(GoogleImageProviderOptions::default());
        };
        let Some(mut obj) = value.as_object().cloned() else {
            return Err(DittoError::invalid_response_text(
                "google image provider_options must be a JSON object".to_string(),
            ));
        };

        for key in [
            "reasoning_effort",
            "response_format",
            "parallel_tool_calls",
            "prompt_cache_key",
        ] {
            if obj.remove(key).is_some() {
                warnings.push(Warning::Unsupported {
                    feature: key.to_string(),
                    details: Some(format!(
                        "Google image generation does not support provider_options.{key}"
                    )),
                });
            }
        }

        serde_json::from_value(Value::Object(obj)).map_err(|err| {
            DittoError::invalid_response_text(format!("invalid google image provider_options: {err}"))
        })
    }

    fn coerce_google_image_size(
        size: Option<&str>,
        warnings: &mut Vec<Warning>,
    ) -> (Option<String>, Option<String>) {
        let Some(size) = size.map(str::trim).filter(|size| !size.is_empty()) else {
            return (None, None);
        };

        if size.contains(':') {
            return (Some(size.to_string()), None);
        }

        let normalized = size.to_ascii_lowercase();
        match normalized.as_str() {
            "1k" => (None, Some("1K".to_string())),
            "2k" => (None, Some("2K".to_string())),
            "4k" => (None, Some("4K".to_string())),
            "1024x1024" => (Some("1:1".to_string()), Some("1K".to_string())),
            "2048x2048" => (Some("1:1".to_string()), Some("2K".to_string())),
            other => {
                warnings.push(Warning::Unsupported {
                    feature: "image.size".to_string(),
                    details: Some(format!(
                        "Google image generation could not map size={other:?}; use provider_options.google.aspect_ratio or image_size"
                    )),
                });
                (None, None)
            }
        }
    }

    fn number_from_f32(value: f32, field: &str) -> Result<Number> {
        Number::from_f64(value as f64).ok_or_else(|| {
            DittoError::invalid_response_text(format!(
                "invalid google image provider_options.{field}: not a finite number"
            ))
        })
    }

    #[async_trait]
    impl ImageGenerationModel for GoogleImages {
        fn provider(&self) -> &str {
            "google"
        }

        fn model_id(&self) -> &str {
            self.client.default_model.as_str()
        }

        async fn generate(
            &self,
            request: ImageGenerationRequest,
        ) -> Result<ImageGenerationResponse> {
            let model = self.resolve_model(&request)?.to_string();
            let selected_provider_options =
                select_provider_options_value(request.provider_options.as_ref(), self.provider())?;

            let mut warnings = Vec::<Warning>::new();
            let provider_options =
                parse_google_image_provider_options(selected_provider_options, &mut warnings)?;
            let (size_aspect_ratio, size_image_size) =
                coerce_google_image_size(request.size.as_deref(), &mut warnings);

            let mut instance = Map::<String, Value>::new();
            instance.insert("prompt".to_string(), Value::String(request.prompt.clone()));

            let mut parameters = Map::<String, Value>::new();
            if let Some(number_of_images) = provider_options.number_of_images.or(request.n) {
                parameters.insert(
                    "sampleCount".to_string(),
                    Value::Number(Number::from(number_of_images)),
                );
            }
            if let Some(aspect_ratio) = provider_options.aspect_ratio.or(size_aspect_ratio) {
                parameters.insert("aspectRatio".to_string(), Value::String(aspect_ratio));
            }
            if let Some(guidance_scale) = provider_options.guidance_scale {
                parameters.insert(
                    "guidanceScale".to_string(),
                    Value::Number(number_from_f32(guidance_scale, "guidance_scale")?),
                );
            }
            if let Some(safety_filter_level) = provider_options.safety_filter_level {
                parameters.insert(
                    "safetySetting".to_string(),
                    Value::String(safety_filter_level),
                );
            }
            if let Some(person_generation) = provider_options.person_generation {
                parameters.insert(
                    "personGeneration".to_string(),
                    Value::String(person_generation),
                );
            }
            if let Some(include_safety_attributes) = provider_options.include_safety_attributes {
                parameters.insert(
                    "includeSafetyAttributes".to_string(),
                    Value::Bool(include_safety_attributes),
                );
            }
            if let Some(include_rai_reason) = provider_options.include_rai_reason {
                parameters.insert(
                    "includeRaiReason".to_string(),
                    Value::Bool(include_rai_reason),
                );
            }
            if let Some(language) = provider_options.language {
                parameters.insert("language".to_string(), Value::String(language));
            }
            if let Some(output_mime_type) = provider_options.output_mime_type {
                parameters.insert(
                    "outputOptions".to_string(),
                    serde_json::json!({ "mimeType": output_mime_type }),
                );
            }
            if let Some(output_compression_quality) = provider_options.output_compression_quality {
                let output_options = parameters
                    .entry("outputOptions".to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Value::Object(output_options) = output_options else {
                    return Err(DittoError::invalid_response_text(
                        "google image outputOptions must be an object".to_string(),
                    ));
                };
                output_options.insert(
                    "compressionQuality".to_string(),
                    Value::Number(Number::from(output_compression_quality)),
                );
            }
            if let Some(image_size) = provider_options.image_size.or(size_image_size) {
                parameters.insert("sampleImageSize".to_string(), Value::String(image_size));
            }

            let mut body = Map::<String, Value>::new();
            body.insert(
                "instances".to_string(),
                Value::Array(vec![Value::Object(instance)]),
            );
            if !parameters.is_empty() {
                body.insert("parameters".to_string(), Value::Object(parameters));
            }

            let raw = crate::provider_transport::send_checked_json::<Value>(
                self.client
                    .apply_auth(self.client.http.post(self.predict_url(&model)))
                    .json(&body),
            )
            .await?;
            let parsed = serde_json::from_value::<GoogleImagePredictResponse>(raw.clone())?;

            let mut images = Vec::new();
            let mut response_format_mismatch = false;
            for prediction in &parsed.predictions {
                let image = if let Some(url) = prediction.gcs_uri.as_ref() {
                    if request.response_format == Some(ImageResponseFormat::Base64Json) {
                        response_format_mismatch = true;
                    }
                    Some(ImageSource::Url { url: url.clone() })
                } else if let Some(data) = prediction.bytes_base64_encoded.as_ref() {
                    if request.response_format == Some(ImageResponseFormat::Url) {
                        response_format_mismatch = true;
                    }
                    Some(ImageSource::Base64 {
                        media_type: prediction
                            .mime_type
                            .clone()
                            .unwrap_or_else(|| "image/png".to_string()),
                        data: data.clone(),
                    })
                } else {
                    None
                };

                if let Some(image) = image {
                    images.push(image);
                }

                if prediction.rai_filtered_reason.is_some() {
                    warnings.push(Warning::Compatibility {
                        feature: "image.rai_filtered_reason".to_string(),
                        details: prediction
                            .rai_filtered_reason
                            .clone()
                            .unwrap_or_else(|| "image filtered by Google RAI policy".to_string()),
                    });
                }
                if let Some(prompt) = prediction
                    .enhanced_prompt
                    .as_ref()
                    .or(prediction.prompt.as_ref())
                    .filter(|prompt| !prompt.trim().is_empty())
                {
                    warnings.push(Warning::Compatibility {
                        feature: "image.enhanced_prompt".to_string(),
                        details: prompt.clone(),
                    });
                }
            }

            if response_format_mismatch {
                warnings.push(Warning::Unsupported {
                    feature: "image.response_format".to_string(),
                    details: Some(
                        "Google image generation does not guarantee OpenAI-style response_format selection"
                            .to_string(),
                    ),
                });
            }

            let provider_metadata = serde_json::json!({
                "raw": raw,
                "positive_prompt_safety_attributes": parsed.positive_prompt_safety_attributes,
            });

            Ok(ImageGenerationResponse {
                images,
                usage: crate::contracts::Usage::default(),
                warnings,
                provider_metadata: Some(provider_metadata),
            })
        }
    }

    #[cfg(test)]
    mod google_image_tests {
        use super::*;
        use httpmock::{Method::POST, MockServer};

        #[tokio::test]
        async fn generate_images_posts_to_predict_endpoint() -> Result<()> {
            if crate::utils::test_support::should_skip_httpmock() {
                return Ok(());
            }
            let server = MockServer::start_async().await;
            let mock = server
                .mock_async(|when, then| {
                    when.method(POST)
                        .path("/v1beta/models/imagen-4:predict")
                        .body_includes("sunlit balcony")
                        .body_includes("sampleCount")
                        .body_includes("aspectRatio");
                    then.status(200)
                        .header("content-type", "application/json")
                        .body(
                            serde_json::json!({
                                "predictions": [{
                                    "bytesBase64Encoded": "AQID",
                                    "mimeType": "image/png"
                                }]
                            })
                            .to_string(),
                        );
                })
                .await;

            let client = GoogleImages::new("")
                .with_base_url(server.url("/v1beta"))
                .with_model("imagen-4");

            let response = client
                .generate(ImageGenerationRequest {
                    prompt: "sunlit balcony".to_string(),
                    model: None,
                    n: Some(2),
                    size: Some("16:9".to_string()),
                    response_format: Some(ImageResponseFormat::Base64Json),
                    provider_options: None,
                })
                .await?;

            mock.assert_async().await;
            assert_eq!(response.images.len(), 1);
            match &response.images[0] {
                ImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "AQID");
                }
                other => panic!("unexpected image source: {other:?}"),
            }
            Ok(())
        }
    }
}

#[cfg(any(feature = "cap-image-generation", feature = "cap-image-edit"))]
pub use google_images_impl::GoogleImages;
