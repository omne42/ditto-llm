#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::DittoError;
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouterConfig, TranslationBackend,
};
use ditto_llm::model::{LanguageModel, StreamResult};
use ditto_llm::types::{FinishReason, GenerateRequest, GenerateResponse, StreamChunk, Usage};
use futures_util::{StreamExt, stream};
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
        Err(DittoError::InvalidResponse(
            "FakeModel.generate should not be called".to_string(),
        ))
    }

    async fn stream(&self, _request: GenerateRequest) -> ditto_llm::Result<StreamResult> {
        let chunks = vec![
            Ok(StreamChunk::ResponseId {
                id: "chatcmpl_fake".to_string(),
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
        Ok(stream::iter(chunks).boxed())
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
async fn gateway_translation_chat_completions_stream_include_usage() -> ditto_llm::Result<()> {
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

    let payload = json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "stream_options": { "include_usage": true },
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
    assert!(text.contains("\"usage\""));
    assert!(text.contains("\"prompt_tokens\""));
    assert!(text.contains("[DONE]"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_chat_completions_stream_defaults_without_usage()
-> ditto_llm::Result<()> {
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
    assert!(!text.contains("\"usage\""));
    assert!(text.contains("[DONE]"));

    Ok(())
}
