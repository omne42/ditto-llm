#[cfg(feature = "cap-video-generation")]
mod google_videos_impl {
    use async_trait::async_trait;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use serde::{Deserialize, Serialize};
    use serde_json::{Map, Number, Value};

    use super::Google;
    use crate::capabilities::file::FileContent;
    use crate::capabilities::video::VideoGenerationModel;
    use crate::config::{Env, ProviderConfig};
    use crate::contracts::Warning;
    use crate::provider_options::select_provider_options_value;
    use crate::types::{
        VideoContentVariant, VideoDeleteResponse, VideoGenerationFailure, VideoGenerationRequest,
        VideoGenerationResponse, VideoGenerationStatus, VideoListRequest, VideoListResponse,
        VideoReferenceUpload, VideoRemixRequest,
    };
    use crate::error::Result;

    #[derive(Clone)]
    pub struct GoogleVideos {
        client: Google,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct GoogleVideoProviderOptions {
        aspect_ratio: Option<String>,
        resolution: Option<String>,
        person_generation: Option<String>,
        negative_prompt: Option<String>,
        enhance_prompt: Option<bool>,
        number_of_videos: Option<u32>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct GoogleOperationFailurePayload {
        #[serde(default)]
        code: Option<Value>,
        #[serde(default)]
        message: Option<String>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct GoogleVideoOperation {
        #[serde(default)]
        name: String,
        #[serde(default)]
        done: bool,
        #[serde(default)]
        metadata: Option<Value>,
        #[serde(default)]
        error: Option<GoogleOperationFailurePayload>,
        #[serde(default)]
        response: Option<GoogleVideoOperationResponse>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct GoogleVideoOperationResponse {
        #[serde(default, rename = "generateVideoResponse")]
        generate_video_response: Option<GoogleVideoResult>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct GoogleVideoResult {
        #[serde(default, rename = "generatedSamples")]
        generated_samples: Vec<GoogleGeneratedVideo>,
        #[serde(default, rename = "raiMediaFilteredCount")]
        rai_media_filtered_count: Option<u32>,
        #[serde(default, rename = "raiMediaFilteredReasons")]
        rai_media_filtered_reasons: Option<Vec<String>>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct GoogleGeneratedVideo {
        #[serde(default)]
        video: Option<GoogleVideoBlob>,
    }

    #[derive(Debug, Default, Deserialize, Serialize, Clone)]
    struct GoogleVideoBlob {
        #[serde(default)]
        uri: Option<String>,
        #[serde(default, rename = "encodedVideo")]
        encoded_video: Option<String>,
        #[serde(default, rename = "encoding")]
        encoding: Option<String>,
    }

    impl GoogleVideos {
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

        fn resolve_model<'a>(&'a self, request: &'a VideoGenerationRequest) -> Result<&'a str> {
            crate::providers::resolve_model_or_default(
                request.model.as_deref().filter(|model| !model.trim().is_empty()),
                self.client.default_model.as_str(),
                "google video",
                "set request.model or GoogleVideos::with_model",
            )
        }

        fn predict_long_running_url(&self, model: &str) -> String {
            let path = Google::model_path(model);
            http_kit::join_api_base_url_path(
                &self.client.base_url,
                &format!("{path}:predictLongRunning"),
            )
        }

        fn operation_url(&self, operation_name: &str) -> Result<String> {
            let operation_name = operation_name.trim();
            if operation_name.is_empty() {
                return Err(crate::invalid_response!(
                    "error_detail.google.video.operation_name_empty"
                ));
            }
            if operation_name.starts_with("http://") || operation_name.starts_with("https://") {
                return Ok(operation_name.to_string());
            }
            Ok(http_kit::join_api_base_url_path(
                &self.client.base_url,
                operation_name,
            ))
        }
    }

    fn parse_google_video_provider_options(
        value: Option<Value>,
        warnings: &mut Vec<Warning>,
    ) -> Result<GoogleVideoProviderOptions> {
        let Some(value) = value else {
            return Ok(GoogleVideoProviderOptions::default());
        };
        let Some(mut obj) = value.as_object().cloned() else {
            return Err(crate::invalid_response!(
                "error_detail.google.video.provider_options_not_object"
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
                        "Google video generation does not support provider_options.{key}"
                    )),
                });
            }
        }

        serde_json::from_value(Value::Object(obj)).map_err(|err| {
            crate::invalid_response!(
                "error_detail.google.video.provider_options_invalid",
                "error" => err.to_string()
            )
        })
    }

    fn infer_google_video_input(upload: &VideoReferenceUpload) -> Result<(String, Value)> {
        let media_type = upload.media_type.clone().unwrap_or_else(|| {
            let filename = upload.filename.to_ascii_lowercase();
            if filename.ends_with(".png") {
                "image/png".to_string()
            } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
                "image/jpeg".to_string()
            } else if filename.ends_with(".webp") {
                "image/webp".to_string()
            } else if filename.ends_with(".mp4") {
                "video/mp4".to_string()
            } else if filename.ends_with(".mov") {
                "video/quicktime".to_string()
            } else {
                "application/octet-stream".to_string()
            }
        });

        let field = if media_type.starts_with("image/") {
            "image"
        } else if media_type.starts_with("video/") {
            "video"
        } else {
            return Err(crate::invalid_response!(
                "error_detail.google.video.input_reference_media_type_invalid",
                "media_type" => format!("{media_type:?}")
            ));
        };

        Ok((
            field.to_string(),
            serde_json::json!({
                "bytesBase64Encoded": BASE64.encode(&upload.data),
                "mimeType": media_type,
            }),
        ))
    }

    fn infer_google_video_size(
        size: Option<&str>,
        warnings: &mut Vec<Warning>,
    ) -> (Option<String>, Option<String>) {
        let Some(size) = size.map(str::trim).filter(|size| !size.is_empty()) else {
            return (None, None);
        };

        if size.contains(':') {
            return (Some(size.to_string()), None);
        }
        if size.ends_with('p') || size.ends_with('P') {
            return (None, Some(size.to_ascii_lowercase()));
        }

        let normalized = size.to_ascii_lowercase();
        let parsed = match normalized.as_str() {
            "1280x720" => Some(("16:9".to_string(), "720p".to_string())),
            "720x1280" => Some(("9:16".to_string(), "720p".to_string())),
            "1920x1080" => Some(("16:9".to_string(), "1080p".to_string())),
            "1080x1920" => Some(("9:16".to_string(), "1080p".to_string())),
            "1024x1024" => Some(("1:1".to_string(), "1024p".to_string())),
            _ => None,
        };

        if let Some((aspect_ratio, resolution)) = parsed {
            return (Some(aspect_ratio), Some(resolution));
        }

        warnings.push(Warning::Unsupported {
            feature: "video.size".to_string(),
            details: Some(format!(
                "Google video generation could not map size={normalized:?}; use provider_options.google.aspect_ratio or resolution"
            )),
        });
        (None, None)
    }

    fn operation_status(operation: &GoogleVideoOperation) -> VideoGenerationStatus {
        if operation.done {
            if operation.error.is_some() {
                return VideoGenerationStatus::Failed;
            }
            return VideoGenerationStatus::Completed;
        }

        let state = operation
            .metadata
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("state").or_else(|| metadata.get("status")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_uppercase();

        if state.contains("QUEUED") || state.contains("PENDING") {
            VideoGenerationStatus::Queued
        } else {
            VideoGenerationStatus::InProgress
        }
    }

    fn operation_progress(operation: &GoogleVideoOperation) -> Option<u32> {
        let metadata = operation.metadata.as_ref()?.as_object()?;
        for key in ["progressPercentage", "progressPercent", "progress"] {
            if let Some(value) = metadata.get(key).and_then(Value::as_u64)
                && let Ok(value) = u32::try_from(value)
            {
                return Some(value);
            }
        }
        None
    }

    fn operation_failure(operation: &GoogleVideoOperation) -> Option<VideoGenerationFailure> {
        let error = operation.error.as_ref()?;
        let code = error
            .code
            .as_ref()
            .map(|code| match code {
                Value::String(code) => code.clone(),
                Value::Number(code) => code.to_string(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "unknown".to_string());
        let message = error
            .message
            .clone()
            .unwrap_or_else(|| "video generation failed".to_string());
        Some(VideoGenerationFailure { code, message })
    }

    fn first_generated_video_blob(operation: &GoogleVideoOperation) -> Option<GoogleVideoBlob> {
        operation
            .response
            .as_ref()?
            .generate_video_response
            .as_ref()?
            .generated_samples
            .iter()
            .find_map(|sample| sample.video.clone())
    }

    fn response_warnings(operation: &GoogleVideoOperation) -> Vec<Warning> {
        let mut warnings = Vec::new();
        if let Some(result) = operation
            .response
            .as_ref()
            .and_then(|response| response.generate_video_response.as_ref())
        {
            if let Some(filtered_count) = result.rai_media_filtered_count
                && filtered_count > 0
            {
                warnings.push(Warning::Compatibility {
                    feature: "video.rai_media_filtered".to_string(),
                    details: format!("Google filtered {filtered_count} generated video sample(s)"),
                });
            }
            if let Some(reasons) = result
                .rai_media_filtered_reasons
                .as_ref()
                .filter(|reasons| !reasons.is_empty())
            {
                warnings.push(Warning::Compatibility {
                    feature: "video.rai_media_filtered_reasons".to_string(),
                    details: reasons.join(", "),
                });
            }
        }
        warnings
    }

    fn operation_to_response(
        operation: GoogleVideoOperation,
        model: Option<String>,
        prompt: Option<String>,
        seconds: Option<u32>,
        size: Option<String>,
    ) -> VideoGenerationResponse {
        let mut warnings = response_warnings(&operation);
        let mut provider_metadata = serde_json::to_value(&operation).ok();
        if let Some(raw) = provider_metadata.as_mut().and_then(Value::as_object_mut) {
            if let Some(model) = model.as_ref() {
                raw.entry("requested_model".to_string())
                    .or_insert_with(|| Value::String(model.clone()));
            }
            if let Some(prompt) = prompt.as_ref() {
                raw.entry("requested_prompt".to_string())
                    .or_insert_with(|| Value::String(prompt.clone()));
            }
        }

        let status = operation_status(&operation);
        let mut response = VideoGenerationResponse {
            id: operation.name.clone(),
            object: Some("google.video.operation".to_string()),
            status,
            model,
            created_at: None,
            completed_at: None,
            expires_at: None,
            progress: operation_progress(&operation),
            prompt,
            remixed_from_video_id: None,
            seconds: seconds.map(|value| value.to_string()),
            size,
            error: operation_failure(&operation),
            warnings: Vec::new(),
            provider_metadata,
        };

        if status == VideoGenerationStatus::Failed && response.error.is_none() {
            response.error = Some(VideoGenerationFailure {
                code: "unknown".to_string(),
                message: "video generation failed".to_string(),
            });
        }
        response.warnings.append(&mut warnings);
        response
    }

    #[async_trait]
    impl VideoGenerationModel for GoogleVideos {
        fn provider(&self) -> &str {
            "google"
        }

        fn model_id(&self) -> &str {
            self.client.default_model.as_str()
        }

        async fn create(&self, request: VideoGenerationRequest) -> Result<VideoGenerationResponse> {
            let model = self.resolve_model(&request)?.to_string();
            let selected_provider_options =
                select_provider_options_value(request.provider_options.as_ref(), self.provider())?;
            let mut warnings = Vec::<Warning>::new();
            let provider_options =
                parse_google_video_provider_options(selected_provider_options, &mut warnings)?;
            let (size_aspect_ratio, size_resolution) =
                infer_google_video_size(request.size.as_deref(), &mut warnings);

            let mut instance = Map::<String, Value>::new();
            instance.insert("prompt".to_string(), Value::String(request.prompt.clone()));
            if let Some(input_reference) = request.input_reference.as_ref() {
                let (field, value) = infer_google_video_input(input_reference)?;
                instance.insert(field, value);
            }

            let mut parameters = Map::<String, Value>::new();
            if let Some(number_of_videos) = provider_options.number_of_videos {
                parameters.insert(
                    "sampleCount".to_string(),
                    Value::Number(Number::from(number_of_videos)),
                );
            }
            if let Some(duration_seconds) = request.seconds {
                parameters.insert(
                    "durationSeconds".to_string(),
                    Value::Number(Number::from(duration_seconds)),
                );
            }
            if let Some(aspect_ratio) = provider_options.aspect_ratio.or(size_aspect_ratio) {
                parameters.insert("aspectRatio".to_string(), Value::String(aspect_ratio));
            }
            if let Some(resolution) = provider_options.resolution.or(size_resolution) {
                parameters.insert("resolution".to_string(), Value::String(resolution));
            }
            if let Some(person_generation) = provider_options.person_generation {
                parameters.insert(
                    "personGeneration".to_string(),
                    Value::String(person_generation),
                );
            }
            if let Some(negative_prompt) = provider_options.negative_prompt {
                parameters.insert("negativePrompt".to_string(), Value::String(negative_prompt));
            }
            if let Some(enhance_prompt) = provider_options.enhance_prompt {
                parameters.insert("enhancePrompt".to_string(), Value::Bool(enhance_prompt));
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
                    .apply_auth(self.client.http.post(self.predict_long_running_url(&model)))
                    .json(&body),
            )
            .await?;
            let operation = serde_json::from_value::<GoogleVideoOperation>(raw)?;
            let mut response = operation_to_response(
                operation,
                Some(model),
                Some(request.prompt),
                request.seconds,
                request.size,
            );
            response.warnings.extend(warnings);
            Ok(response)
        }

        async fn retrieve(&self, video_id: &str) -> Result<VideoGenerationResponse> {
            let raw = crate::provider_transport::send_checked_json::<Value>(
                self.client
                    .apply_auth(self.client.http.get(self.operation_url(video_id)?)),
            )
            .await?;
            let operation = serde_json::from_value::<GoogleVideoOperation>(raw)?;
            Ok(operation_to_response(operation, None, None, None, None))
        }

        async fn download_content(
            &self,
            video_id: &str,
            variant: Option<VideoContentVariant>,
        ) -> Result<FileContent> {
            match variant.unwrap_or(VideoContentVariant::Video) {
                VideoContentVariant::Video => {}
                other => {
                    return Err(crate::invalid_response!(
                        "error_detail.google.video.content_variant_unsupported",
                        "variant" => format!("{other:?}")
                    ));
                }
            }

            let response = self.retrieve(video_id).await?;
            let raw = response.provider_metadata.ok_or_else(|| {
                crate::invalid_response!(
                    "error_detail.google.video.response_missing_provider_metadata"
                )
            })?;
            let operation = serde_json::from_value::<GoogleVideoOperation>(raw)?;
            let video = first_generated_video_blob(&operation).ok_or_else(|| {
                crate::invalid_response!(
                    "error_detail.google.video.operation_missing_generated_video_content"
                )
            })?;

            if let Some(encoded_video) = video.encoded_video.as_ref() {
                let bytes = BASE64.decode(encoded_video).map_err(|err| {
                    crate::invalid_response!(
                        "error_detail.google.video.encoded_video_invalid",
                        "error" => err.to_string()
                    )
                })?;
                return Ok(FileContent {
                    bytes,
                    media_type: video.encoding,
                });
            }

            let uri = video.uri.ok_or_else(|| {
                crate::invalid_response!(
                    "error_detail.google.video.operation_missing_uri_and_encoded_video"
                )
            })?;
            if !(uri.starts_with("http://") || uri.starts_with("https://")) {
                return Err(crate::invalid_response!(
                    "error_detail.google.video.download_uri_non_http",
                    "uri" => format!("{uri:?}")
                ));
            }

            let bytes =
                crate::provider_transport::send_checked_bytes(self.client.http.get(&uri)).await?;
            Ok(FileContent {
                bytes: bytes.to_vec(),
                media_type: video.encoding,
            })
        }

        async fn delete(&self, video_id: &str) -> Result<VideoDeleteResponse> {
            let _ = video_id;
            Err(crate::invalid_response!(
                "error_detail.google.video.operation_unsupported",
                "operation" => "delete"
            ))
        }

        async fn list(&self, request: VideoListRequest) -> Result<VideoListResponse> {
            let _ = request;
            Err(crate::invalid_response!(
                "error_detail.google.video.operation_unsupported",
                "operation" => "list"
            ))
        }

        async fn remix(
            &self,
            video_id: &str,
            request: VideoRemixRequest,
        ) -> Result<VideoGenerationResponse> {
            let _ = video_id;
            let _ = request;
            Err(crate::invalid_response!(
                "error_detail.google.video.operation_unsupported",
                "operation" => "remix"
            ))
        }
    }

    #[cfg(test)]
    mod google_video_tests {
        use super::*;
        use httpmock::{Method::GET, Method::POST, MockServer};

        #[test]
        fn operation_url_joins_relative_name_against_base_url() -> Result<()> {
            let client = GoogleVideos::new("")
                .with_base_url("https://proxy.example/v1beta")
                .with_model("veo-2.0-generate-001");
            assert_eq!(
                client.operation_url("/operations/video-123")?,
                "https://proxy.example/v1beta/operations/video-123"
            );
            Ok(())
        }

        #[tokio::test]
        async fn create_video_posts_to_predict_long_running_endpoint() -> Result<()> {
            if crate::utils::test_support::should_skip_httpmock() {
                return Ok(());
            }
            let server = MockServer::start_async().await;
            let create_mock = server
                .mock_async(|when, then| {
                    when.method(POST)
                        .path("/v1beta/models/veo-2.0-generate-001:predictLongRunning")
                        .body_includes("A neon cat")
                        .body_includes("durationSeconds")
                        .body_includes("aspectRatio");
                    then.status(200)
                        .header("content-type", "application/json")
                        .body(
                            serde_json::json!({
                                "name": "operations/video-123",
                                "done": false,
                                "metadata": {
                                    "state": "QUEUED",
                                    "progressPercentage": 0
                                }
                            })
                            .to_string(),
                        );
                })
                .await;

            let client = GoogleVideos::new("")
                .with_base_url(server.url("/v1beta"))
                .with_model("veo-2.0-generate-001");
            let response = client
                .create(VideoGenerationRequest {
                    prompt: "A neon cat".to_string(),
                    input_reference: None,
                    model: None,
                    seconds: Some(8),
                    size: Some("16:9".to_string()),
                    provider_options: None,
                })
                .await?;

            create_mock.assert_async().await;
            assert_eq!(response.id, "operations/video-123");
            assert_eq!(response.status, VideoGenerationStatus::Queued);
            assert_eq!(response.progress, Some(0));
            Ok(())
        }

        #[tokio::test]
        async fn retrieve_and_download_google_video_operation() -> Result<()> {
            if crate::utils::test_support::should_skip_httpmock() {
                return Ok(());
            }
            let server = MockServer::start_async().await;
            let download_url = server.url("/download/video-123.mp4");
            let get_operation = server
                .mock_async(move |when, then| {
                    when.method(GET).path("/v1beta/operations/video-123");
                    then.status(200)
                        .header("content-type", "application/json")
                        .body(
                            serde_json::json!({
                                "name": "operations/video-123",
                                "done": true,
                                "response": {
                                    "generateVideoResponse": {
                                        "generatedSamples": [{
                                            "video": {
                                                "uri": download_url.clone(),
                                                "encoding": "video/mp4"
                                            }
                                        }]
                                    }
                                }
                            })
                            .to_string(),
                        );
                })
                .await;
            let get_video = server
                .mock_async(|when, then| {
                    when.method(GET).path("/download/video-123.mp4");
                    then.status(200)
                        .header("content-type", "video/mp4")
                        .body(vec![1u8, 2, 3, 4]);
                })
                .await;

            let client = GoogleVideos::new("")
                .with_base_url(server.url("/v1beta"))
                .with_model("veo-2.0-generate-001");
            let response = client.retrieve("operations/video-123").await?;
            assert_eq!(response.status, VideoGenerationStatus::Completed);

            let content = client
                .download_content("operations/video-123", None)
                .await?;
            get_operation.assert_calls_async(2).await;
            get_video.assert_async().await;
            assert_eq!(content.bytes, vec![1u8, 2, 3, 4]);
            assert_eq!(content.media_type.as_deref(), Some("video/mp4"));
            Ok(())
        }
    }
}

#[cfg(feature = "cap-video-generation")]
pub use google_videos_impl::GoogleVideos;
