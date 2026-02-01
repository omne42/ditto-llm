
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
    GenerateResponse, ImageGenerationRequest, ImageGenerationResponse, ImageSource, Message,
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
                cache_input_tokens: None,
                cache_creation_input_tokens: None,
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

    async fn generate(&self, _request: GenerateRequest) -> ditto_llm::Result<GenerateResponse> {
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

    async fn stream(&self, _request: GenerateRequest) -> ditto_llm::Result<StreamResult> {
        let chunks: Vec<ditto_llm::Result<StreamChunk>> = Vec::new();
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
async fn gateway_translation_completions_non_streaming() -> ditto_llm::Result<()> {
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
async fn gateway_translation_completions_streaming() -> ditto_llm::Result<()> {
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
async fn gateway_translation_models_list() -> ditto_llm::Result<()> {
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
        TranslationBackend::new("other", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

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
        Some("multi")
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
    assert!(ids.contains(&"fake/fake-model".to_string()));
    assert!(ids.contains(&"other/fake-model".to_string()));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_retrieve() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/models/fake/fake-model")
        .body(Body::empty())
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
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("model"));
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("fake/fake-model")
    );
    assert_eq!(
        parsed.get("owned_by").and_then(|v| v.as_str()),
        Some("fake")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_models_retrieve_unknown() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

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

