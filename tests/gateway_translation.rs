#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::embedding::EmbeddingModel;
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouterConfig, TranslationBackend,
};
use ditto_llm::model::{LanguageModel, StreamResult};
use ditto_llm::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, StreamChunk, Usage,
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
            .with_embedding_model(Arc::new(FakeEmbeddingModel)),
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
            .with_embedding_model(Arc::new(FakeEmbeddingModel)),
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
            .with_embedding_model(Arc::new(FakeEmbeddingModel)),
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
            .with_embedding_model(Arc::new(FakeEmbeddingModel)),
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
            .with_embedding_model(Arc::new(FakeEmbeddingModel)),
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
