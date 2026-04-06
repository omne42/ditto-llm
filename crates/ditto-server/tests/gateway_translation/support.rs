use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Request, StatusCode};
use ditto_core::capabilities::BatchClient;
use ditto_core::capabilities::audio::{AudioTranscriptionModel, SpeechModel};
use ditto_core::capabilities::embedding::EmbeddingModel;
use ditto_core::capabilities::file::FileContent;
use ditto_core::capabilities::video::VideoGenerationModel;
use ditto_core::capabilities::{
    ImageEditModel, ImageGenerationModel, ModerationModel, RerankModel,
};
#[allow(unused_imports)]
use ditto_core::contracts::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, ImageSource, Message,
    StreamChunk, Usage,
};
use ditto_core::llm_core::model::{LanguageModel, StreamResult};
use ditto_core::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, BatchStatus, ImageEditRequest, ImageEditResponse,
    ImageGenerationRequest, ImageGenerationResponse, ImageResponseFormat, ModerationInput,
    ModerationRequest, ModerationResponse, ModerationResult, RerankRequest, RerankResponse,
    RerankResult, SpeechRequest, SpeechResponse, VideoContentVariant, VideoDeleteResponse,
    VideoGenerationRequest, VideoGenerationResponse, VideoGenerationStatus, VideoListOrder,
    VideoListRequest, VideoListResponse, VideoRemixRequest,
};
use ditto_server::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouteBackend, RouterConfig, TranslationBackend,
};
use futures_util::StreamExt;
use serde_json::json;
use tower::util::ServiceExt;

#[derive(Clone)]
struct FakeModel;

#[async_trait]
impl LanguageModel for FakeModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }

    async fn generate(
        &self,
        _request: GenerateRequest,
    ) -> ditto_core::error::Result<GenerateResponse> {
        Ok(GenerateResponse {
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: Some(1),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(2),
                total_tokens: Some(3),
            },
            warnings: Vec::new(),
            provider_metadata: Some(json!({ "id": "resp_fake" })),
        })
    }

    async fn stream(&self, _request: GenerateRequest) -> ditto_core::error::Result<StreamResult> {
        let chunks = vec![
            Ok(StreamChunk::ResponseId {
                id: "resp_fake".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Ok(StreamChunk::FinishReason(FinishReason::Stop)),
            Ok(StreamChunk::Usage(Usage {
                input_tokens: Some(1),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(2),
                total_tokens: Some(3),
            })),
        ];
        Ok(futures_util::stream::iter(chunks).boxed())
    }
}

#[derive(Clone)]
struct FakeCompactionModel;

#[async_trait]
impl LanguageModel for FakeCompactionModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-compaction"
    }

    async fn generate(
        &self,
        _request: GenerateRequest,
    ) -> ditto_core::error::Result<GenerateResponse> {
        Ok(GenerateResponse {
            content: vec![ContentPart::ToolCall {
                id: "call_0".to_string(),
                name: "__ditto_object__".to_string(),
                arguments: json!({
                    "value": [
                        {
                            "type": "message",
                            "role": "user",
                            "content": [{"type":"input_text","text":"compacted"}],
                        }
                    ]
                }),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: Some(1),
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
                output_tokens: Some(2),
                total_tokens: Some(3),
            },
            warnings: Vec::new(),
            provider_metadata: Some(json!({ "id": "resp_fake_compact" })),
        })
    }

    async fn stream(&self, _request: GenerateRequest) -> ditto_core::error::Result<StreamResult> {
        let chunks: Vec<ditto_core::error::Result<StreamChunk>> = Vec::new();
        Ok(futures_util::stream::iter(chunks).boxed())
    }
}

#[derive(Clone)]
struct FakeEmbeddingModel;

#[async_trait]
impl EmbeddingModel for FakeEmbeddingModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-embed"
    }

    async fn embed(&self, texts: Vec<String>) -> ditto_core::error::Result<Vec<Vec<f32>>> {
        Ok(texts
            .into_iter()
            .enumerate()
            .map(|(idx, _)| vec![idx as f32, 0.5])
            .collect())
    }
}

#[derive(Clone)]
struct FakeModerationModel;

#[async_trait]
impl ModerationModel for FakeModerationModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-moderation"
    }

    async fn moderate(
        &self,
        request: ModerationRequest,
    ) -> ditto_core::error::Result<ModerationResponse> {
        let flagged = matches!(request.input, ModerationInput::Text(ref text) if text == "bad");
        Ok(ModerationResponse {
            id: Some("modr_fake".to_string()),
            model: Some(
                request
                    .model
                    .unwrap_or_else(|| "omni-moderation-latest".to_string()),
            ),
            results: vec![ModerationResult {
                flagged,
                categories: Default::default(),
                category_scores: Default::default(),
                provider_metadata: None,
            }],
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeImageModel;

#[async_trait]
impl ImageGenerationModel for FakeImageModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-image"
    }

    async fn generate(
        &self,
        _request: ImageGenerationRequest,
    ) -> ditto_core::error::Result<ImageGenerationResponse> {
        Ok(ImageGenerationResponse {
            images: vec![
                ImageSource::Url {
                    url: "https://example.com/image.png".to_string(),
                },
                ImageSource::Base64 {
                    media_type: "image/png".to_string(),
                    data: "aGVsbG8=".to_string(),
                },
            ],
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeImageEditModel;

#[async_trait]
impl ImageEditModel for FakeImageEditModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-image-edit"
    }

    async fn edit(
        &self,
        request: ImageEditRequest,
    ) -> ditto_core::error::Result<ImageEditResponse> {
        assert_eq!(request.prompt, "remove background");
        assert_eq!(request.model.as_deref(), Some("image-edit-v2"));
        assert_eq!(request.n, Some(2));
        assert_eq!(request.size.as_deref(), Some("1024x1024"));
        assert_eq!(
            request.response_format,
            Some(ImageResponseFormat::Base64Json)
        );
        assert_eq!(request.images.len(), 1);
        assert_eq!(request.images[0].filename, "image.png");
        assert_eq!(request.images[0].media_type.as_deref(), Some("image/png"));
        assert_eq!(request.images[0].data, b"image-bytes".to_vec());
        let mask = request.mask.expect("mask must be present");
        assert_eq!(mask.filename, "mask.png");
        assert_eq!(mask.media_type.as_deref(), Some("image/png"));
        assert_eq!(mask.data, b"mask-bytes".to_vec());

        Ok(ImageGenerationResponse {
            images: vec![
                ImageSource::Url {
                    url: "https://example.com/edited.png".to_string(),
                },
                ImageSource::Base64 {
                    media_type: "image/png".to_string(),
                    data: "ZWRpdGVk".to_string(),
                },
            ],
            usage: Usage::default(),
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeVideoGenerationModel;

#[async_trait]
impl VideoGenerationModel for FakeVideoGenerationModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-video"
    }

    async fn create(
        &self,
        request: VideoGenerationRequest,
    ) -> ditto_core::error::Result<VideoGenerationResponse> {
        match request.prompt.as_str() {
            "road at dusk" => {
                assert!(request.input_reference.is_none());
                assert_eq!(request.model.as_deref(), Some("sora-fast"));
                assert_eq!(request.seconds, Some(4));
                assert_eq!(request.size.as_deref(), Some("1280x720"));
                Ok(VideoGenerationResponse {
                    id: "vid_json".to_string(),
                    object: Some("video".to_string()),
                    status: VideoGenerationStatus::Queued,
                    model: request.model,
                    created_at: Some(123),
                    prompt: Some("road at dusk".to_string()),
                    seconds: Some("4".to_string()),
                    size: Some("1280x720".to_string()),
                    ..Default::default()
                })
            }
            "remix shot" => {
                let input = request.input_reference.expect("input_reference must exist");
                assert_eq!(input.filename, "shot.mp4");
                assert_eq!(input.media_type.as_deref(), Some("video/mp4"));
                assert_eq!(input.data, b"video-bytes".to_vec());
                assert_eq!(request.model.as_deref(), Some("sora-fast"));
                assert_eq!(request.seconds, Some(6));
                assert_eq!(request.size.as_deref(), Some("720p"));
                Ok(VideoGenerationResponse {
                    id: "vid_multipart".to_string(),
                    object: Some("video".to_string()),
                    status: VideoGenerationStatus::Queued,
                    model: request.model,
                    created_at: Some(456),
                    prompt: Some("remix shot".to_string()),
                    seconds: Some("6".to_string()),
                    size: Some("720p".to_string()),
                    ..Default::default()
                })
            }
            other => Err(ditto_core::invalid_response!(
                "error_detail.freeform",
                "message" => format!("unexpected video prompt: {other}")
            )),
        }
    }

    async fn retrieve(&self, video_id: &str) -> ditto_core::error::Result<VideoGenerationResponse> {
        assert_eq!(video_id, "vid_123");
        Ok(VideoGenerationResponse {
            id: "vid_123".to_string(),
            object: Some("video".to_string()),
            status: VideoGenerationStatus::Completed,
            model: Some("sora-fast".to_string()),
            prompt: Some("road at dusk".to_string()),
            progress: Some(100),
            completed_at: Some(789),
            ..Default::default()
        })
    }

    async fn list(
        &self,
        request: VideoListRequest,
    ) -> ditto_core::error::Result<VideoListResponse> {
        assert_eq!(request.limit, Some(2));
        assert_eq!(request.after.as_deref(), Some("vid_111"));
        assert_eq!(request.order, Some(VideoListOrder::Desc));
        Ok(VideoListResponse {
            videos: vec![VideoGenerationResponse {
                id: "vid_123".to_string(),
                object: Some("video".to_string()),
                status: VideoGenerationStatus::Completed,
                model: Some("sora-fast".to_string()),
                prompt: Some("road at dusk".to_string()),
                ..Default::default()
            }],
            after: Some("vid_123".to_string()),
            has_more: Some(false),
            ..Default::default()
        })
    }

    async fn delete(&self, video_id: &str) -> ditto_core::error::Result<VideoDeleteResponse> {
        assert_eq!(video_id, "vid_123");
        Ok(VideoDeleteResponse {
            id: "vid_123".to_string(),
            deleted: true,
            object: Some("video.deleted".to_string()),
        })
    }

    async fn download_content(
        &self,
        video_id: &str,
        variant: Option<VideoContentVariant>,
    ) -> ditto_core::error::Result<FileContent> {
        assert_eq!(video_id, "vid_123");
        assert_eq!(variant, Some(VideoContentVariant::Thumbnail));
        Ok(FileContent {
            bytes: b"thumbnail-bytes".to_vec(),
            media_type: Some("image/png".to_string()),
        })
    }

    async fn remix(
        &self,
        video_id: &str,
        request: VideoRemixRequest,
    ) -> ditto_core::error::Result<VideoGenerationResponse> {
        assert_eq!(video_id, "vid_123");
        assert_eq!(request.prompt, "change angle");
        assert!(request.provider_options.is_none());
        Ok(VideoGenerationResponse {
            id: "vid_remix".to_string(),
            object: Some("video".to_string()),
            status: VideoGenerationStatus::Queued,
            model: Some("sora-fast".to_string()),
            prompt: Some(request.prompt),
            remixed_from_video_id: Some(video_id.to_string()),
            ..Default::default()
        })
    }
}

#[derive(Clone)]
struct FakeAudioTranscriptionModel;

#[async_trait]
impl AudioTranscriptionModel for FakeAudioTranscriptionModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-audio-transcription"
    }

    async fn transcribe(
        &self,
        request: AudioTranscriptionRequest,
    ) -> ditto_core::error::Result<AudioTranscriptionResponse> {
        let model = request.model.unwrap_or_default();
        Ok(AudioTranscriptionResponse {
            text: format!("transcribed:{model}"),
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeSpeechModel;

#[async_trait]
impl SpeechModel for FakeSpeechModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-speech"
    }

    async fn speak(&self, request: SpeechRequest) -> ditto_core::error::Result<SpeechResponse> {
        let _ = request;
        Ok(SpeechResponse {
            audio: vec![0, 1, 2, 3],
            media_type: None,
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeBatchClient;

#[async_trait]
impl BatchClient for FakeBatchClient {
    fn provider(&self) -> &str {
        "fake"
    }

    async fn create(
        &self,
        request: BatchCreateRequest,
    ) -> ditto_core::error::Result<BatchResponse> {
        Ok(BatchResponse {
            batch: Batch {
                id: "batch_created".to_string(),
                status: BatchStatus::Validating,
                endpoint: Some(request.endpoint),
                completion_window: Some(request.completion_window),
                input_file_id: Some(request.input_file_id),
                ..Default::default()
            },
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }

    async fn retrieve(&self, batch_id: &str) -> ditto_core::error::Result<BatchResponse> {
        Ok(BatchResponse {
            batch: Batch {
                id: batch_id.to_string(),
                status: BatchStatus::InProgress,
                ..Default::default()
            },
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }

    async fn cancel(&self, batch_id: &str) -> ditto_core::error::Result<BatchResponse> {
        Ok(BatchResponse {
            batch: Batch {
                id: batch_id.to_string(),
                status: BatchStatus::Cancelled,
                ..Default::default()
            },
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }

    async fn list(
        &self,
        _limit: Option<u32>,
        _after: Option<String>,
    ) -> ditto_core::error::Result<BatchListResponse> {
        Ok(BatchListResponse {
            batches: vec![
                Batch {
                    id: "batch_1".to_string(),
                    status: BatchStatus::InProgress,
                    ..Default::default()
                },
                Batch {
                    id: "batch_2".to_string(),
                    status: BatchStatus::Completed,
                    output_file_id: Some("file_out".to_string()),
                    ..Default::default()
                },
            ],
            after: Some("batch_2".to_string()),
            has_more: Some(false),
            warnings: Vec::new(),
            provider_metadata: None,
        })
    }
}

#[derive(Clone)]
struct FakeRerankModel;

#[async_trait]
impl RerankModel for FakeRerankModel {
    fn provider(&self) -> &str {
        "fake"
    }

    fn model_id(&self) -> &str {
        "fake-rerank"
    }

    async fn rerank(&self, _request: RerankRequest) -> ditto_core::error::Result<RerankResponse> {
        Ok(RerankResponse {
            ranking: vec![
                RerankResult {
                    index: 0,
                    relevance_score: 0.9,
                    provider_metadata: None,
                },
                RerankResult {
                    index: 1,
                    relevance_score: 0.1,
                    provider_metadata: None,
                },
            ],
            warnings: Vec::new(),
            provider_metadata: Some(json!({
                "id": "rr_fake",
                "meta": { "billed_units": { "search_units": 1 } }
            })),
        })
    }
}

fn base_gateway() -> Gateway {
    Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![ditto_server::gateway::VirtualKeyConfig::new("key-1", "vk-1")],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    })
}

fn authorized_test_app(
    state: GatewayHttpState,
) -> tower::util::BoxCloneService<
    Request<Body>,
    axum::response::Response,
    std::convert::Infallible,
> {
    let app = ditto_server::gateway::http::router(state);
    tower::service_fn(move |mut request: Request<Body>| {
        let app = app.clone();
        async move {
            if !request.headers().contains_key(axum::http::header::AUTHORIZATION) {
                request.headers_mut().insert(
                    axum::http::header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer vk-1"),
                );
            }
            app.oneshot(request).await
        }
    })
    .boxed_clone()
}

#[tokio::test]
async fn gateway_translation_chat_completions_non_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_embedding_model(Arc::new(FakeEmbeddingModel))
            .with_moderation_model(Arc::new(FakeModerationModel))
            .with_image_generation_model(Arc::new(FakeImageModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("resp_fake"));
    assert_eq!(
        parsed
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("message"))
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_chat_completions_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_embedding_model(Arc::new(FakeEmbeddingModel))
            .with_moderation_model(Arc::new(FakeModerationModel))
            .with_image_generation_model(Arc::new(FakeImageModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "messages": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("chat.completion.chunk"));
    assert!(text.contains("[DONE]"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_completions_non_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_embedding_model(Arc::new(FakeEmbeddingModel))
            .with_moderation_model(Arc::new(FakeModerationModel))
            .with_image_generation_model(Arc::new(FakeImageModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "prompt": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/completions")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("resp_fake"));
    assert_eq!(
        parsed.get("object").and_then(|v| v.as_str()),
        Some("text_completion")
    );
    assert_eq!(
        parsed
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str()),
        Some("hello")
    );
    assert_eq!(
        parsed
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("finish_reason"))
            .and_then(|v| v.as_str()),
        Some("stop")
    );
    assert_eq!(
        parsed
            .get("usage")
            .and_then(|v| v.get("total_tokens"))
            .and_then(|v| v.as_u64()),
        Some(3)
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_completions_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_embedding_model(Arc::new(FakeEmbeddingModel))
            .with_moderation_model(Arc::new(FakeModerationModel))
            .with_image_generation_model(Arc::new(FakeImageModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "prompt": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/completions")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("text_completion"));
    assert!(text.contains("\"text\":\"hello\""));
    assert!(text.contains("[DONE]"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_list() -> ditto_core::error::Result<()> {
    use std::collections::BTreeMap;

    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();

    let mut model_map = BTreeMap::new();
    model_map.insert("gpt-4o-mini".to_string(), "fake-model".to_string());

    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)).with_model_map(model_map),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("list"));

    let ids = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .map(|data| {
            data.iter()
                .filter_map(|item| {
                    item.get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    assert!(ids.contains(&"gpt-4o-mini".to_string()));
    assert!(ids.contains(&"primary/fake-model".to_string()));
    assert!(!ids.contains(&"secondary/fake-model".to_string()));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_list_respects_virtual_key_route()
-> ditto_core::error::Result<()> {
    use std::collections::BTreeMap;

    let mut key = ditto_server::gateway::VirtualKeyConfig::new("key-secondary", "vk-secondary");
    key.route = Some("secondary".to_string());
    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    });

    let mut primary_map = BTreeMap::new();
    primary_map.insert("gpt-4o-mini".to_string(), "primary-model".to_string());
    let mut secondary_map = BTreeMap::new();
    secondary_map.insert("gpt-4o-mini".to_string(), "secondary-model".to_string());

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)).with_model_map(primary_map),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)).with_model_map(secondary_map),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-secondary")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("secondary")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    let ids = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .map(|data| {
            data.iter()
                .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    assert!(ids.contains(&"gpt-4o-mini"));
    assert!(ids.contains(&"secondary/secondary-model"));
    assert!(!ids.contains(&"primary/primary-model"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_list_hides_unroutable_translation_backends()
-> ditto_core::error::Result<()> {
    use std::collections::BTreeMap;

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![ditto_server::gateway::VirtualKeyConfig::new(
            "key-primary",
            "vk-primary",
        )],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    });

    let mut primary_map = BTreeMap::new();
    primary_map.insert("gpt-4o-mini".to_string(), "primary-model".to_string());
    let mut secondary_map = BTreeMap::new();
    secondary_map.insert("claude-3.5-sonnet".to_string(), "secondary-model".to_string());

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)).with_model_map(primary_map),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)).with_model_map(secondary_map),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    let ids = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .map(|data| {
            data.iter()
                .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    assert!(ids.contains(&"gpt-4o-mini"));
    assert!(ids.contains(&"primary/primary-model"));
    assert!(!ids.contains(&"claude-3.5-sonnet"));
    assert!(!ids.contains(&"secondary/secondary-model"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_retrieve() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models/primary/fake-model")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("model"));
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("primary/fake-model")
    );
    assert_eq!(
        parsed.get("owned_by").and_then(|v| v.as_str()),
        Some("primary")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_retrieve_unknown() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models/does-not-exist")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_non_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_embedding_model(Arc::new(FakeEmbeddingModel))
            .with_moderation_model(Arc::new(FakeModerationModel))
            .with_image_generation_model(Arc::new(FakeImageModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "input": [{"role":"user","content":"hi"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert!(parsed
        .get("id")
        .and_then(|v| v.as_str())
        .is_some_and(|id| id.starts_with("resp_ditto_")));
    assert_eq!(
        parsed.get("output_text").and_then(|v| v.as_str()),
        Some("hello")
    );

    Ok(())
}
