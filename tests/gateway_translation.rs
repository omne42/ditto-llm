#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::audio::{AudioTranscriptionModel, SpeechModel};
use ditto_llm::batch::BatchClient;
use ditto_llm::embedding::EmbeddingModel;
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouterConfig, TranslationBackend,
};
use ditto_llm::image::ImageGenerationModel;
use ditto_llm::model::{LanguageModel, StreamResult};
use ditto_llm::moderation::ModerationModel;
use ditto_llm::rerank::RerankModel;
use ditto_llm::types::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, Batch, BatchCreateRequest,
    BatchListResponse, BatchResponse, BatchStatus, ContentPart, FinishReason, GenerateRequest,
    GenerateResponse, ImageGenerationRequest, ImageGenerationResponse, ImageSource,
    ModerationInput, ModerationRequest, ModerationResponse, ModerationResult, RerankRequest,
    RerankResponse, RerankResult, SpeechRequest, SpeechResponse, StreamChunk, Usage,
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

    async fn generate(&self, _request: GenerateRequest) -> ditto_llm::Result<GenerateResponse> {
        Ok(GenerateResponse {
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: Some(1),
                output_tokens: Some(2),
                total_tokens: Some(3),
            },
            warnings: Vec::new(),
            provider_metadata: Some(json!({ "id": "resp_fake" })),
        })
    }

    async fn stream(&self, _request: GenerateRequest) -> ditto_llm::Result<StreamResult> {
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
                output_tokens: Some(2),
                total_tokens: Some(3),
            })),
        ];
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

    async fn embed(&self, texts: Vec<String>) -> ditto_llm::Result<Vec<Vec<f32>>> {
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

    async fn moderate(&self, request: ModerationRequest) -> ditto_llm::Result<ModerationResponse> {
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
    ) -> ditto_llm::Result<ImageGenerationResponse> {
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
    ) -> ditto_llm::Result<AudioTranscriptionResponse> {
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

    async fn speak(&self, request: SpeechRequest) -> ditto_llm::Result<SpeechResponse> {
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

    async fn create(&self, request: BatchCreateRequest) -> ditto_llm::Result<BatchResponse> {
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

    async fn retrieve(&self, batch_id: &str) -> ditto_llm::Result<BatchResponse> {
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

    async fn cancel(&self, batch_id: &str) -> ditto_llm::Result<BatchResponse> {
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
    ) -> ditto_llm::Result<BatchListResponse> {
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

    async fn rerank(&self, _request: RerankRequest) -> ditto_llm::Result<RerankResponse> {
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
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    })
}

#[tokio::test]
async fn gateway_translation_chat_completions_non_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

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
async fn gateway_translation_chat_completions_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

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
async fn gateway_translation_responses_non_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

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
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("resp_fake"));
    assert_eq!(
        parsed.get("output_text").and_then(|v| v.as_str()),
        Some("hello")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "stream": true,
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
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"type\":\"response.created\""));
    assert!(text.contains("\"type\":\"response.output_text.delta\""));
    assert!(text.contains("\"type\":\"response.completed\""));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_embeddings_non_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "text-embedding-3-small",
        "input": ["a", "b"]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
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
    assert_eq!(
        parsed.get("model").and_then(|v| v.as_str()),
        Some("text-embedding-3-small")
    );
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("list"));
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(2)
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_moderations_non_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "input": "bad"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/moderations")
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
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("modr_fake"));
    assert_eq!(
        parsed
            .get("results")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("flagged"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_images_generations_non_streaming() -> ditto_llm::Result<()> {
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
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "prompt": "hi"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
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
    assert!(parsed.get("created").and_then(|v| v.as_u64()).is_some());
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(2)
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_audio_transcriptions_non_streaming() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_audio_transcription_model(Arc::new(FakeAudioTranscriptionModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart))
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
    assert_eq!(
        parsed.get("text").and_then(|v| v.as_str()),
        Some("transcribed:whisper-1")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_audio_speech_non_streaming() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_speech_model(Arc::new(FakeSpeechModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "tts-1",
        "input": "hello",
        "voice": "alloy",
        "response_format": "mp3"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("audio/mpeg")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), &[0, 1, 2, 3]);
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_create() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "input_file_id": "file_123",
        "endpoint": "/v1/chat/completions",
        "completion_window": "24h"
    });

    let request = Request::builder()
        .method("POST")
        .uri("/v1/batches")
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
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("batch"));
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("batch_created")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_list() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/batches?limit=2&after=batch_111")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;

    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("list"));
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(2)
    );
    assert_eq!(
        parsed.get("last_id").and_then(|v| v.as_str()),
        Some("batch_2")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_retrieve() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/batches/batch_123")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("batch_123"));
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_cancel() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/batches/batch_123/cancel")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("batch_123"));
    Ok(())
}

#[tokio::test]
async fn gateway_translation_rerank_non_streaming() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_rerank_model(Arc::new(FakeRerankModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "rerank-v3.5",
        "query": "hello",
        "documents": ["a", "b"],
        "top_n": 2
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/rerank")
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
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("rr_fake"));
    assert_eq!(
        parsed
            .get("results")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(2)
    );
    Ok(())
}
