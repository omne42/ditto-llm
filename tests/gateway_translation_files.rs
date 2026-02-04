#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    Gateway, GatewayConfig, GatewayHttpState, RouteBackend, RouterConfig, TranslationBackend,
};
use ditto_llm::model::{LanguageModel, StreamResult};
use ditto_llm::types::{GenerateRequest, GenerateResponse};
use ditto_llm::{
    DittoError, FileClient, FileContent, FileDeleteResponse, FileObject, FileUploadRequest,
};
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
        Err(DittoError::InvalidResponse(
            "FakeModel.stream should not be called".to_string(),
        ))
    }
}

#[derive(Clone)]
struct FakeFileClient;

#[async_trait]
impl FileClient for FakeFileClient {
    fn provider(&self) -> &str {
        "fake"
    }

    async fn upload_file_with_purpose(
        &self,
        request: FileUploadRequest,
    ) -> ditto_llm::Result<String> {
        assert_eq!(request.filename, "hello.txt");
        assert_eq!(request.purpose, "fine-tune");
        assert_eq!(request.bytes, b"hello world".to_vec());
        assert_eq!(request.media_type.as_deref(), Some("text/plain"));
        Ok("file_fake".to_string())
    }

    async fn list_files(&self) -> ditto_llm::Result<Vec<FileObject>> {
        Ok(vec![FileObject {
            id: "file_fake".to_string(),
            bytes: 11,
            created_at: 123,
            filename: "hello.txt".to_string(),
            purpose: "fine-tune".to_string(),
            status: None,
            status_details: None,
        }])
    }

    async fn retrieve_file(&self, file_id: &str) -> ditto_llm::Result<FileObject> {
        assert_eq!(file_id, "file_fake");
        Ok(FileObject {
            id: "file_fake".to_string(),
            bytes: 11,
            created_at: 123,
            filename: "hello.txt".to_string(),
            purpose: "fine-tune".to_string(),
            status: None,
            status_details: None,
        })
    }

    async fn delete_file(&self, file_id: &str) -> ditto_llm::Result<FileDeleteResponse> {
        assert_eq!(file_id, "file_fake");
        Ok(FileDeleteResponse {
            id: file_id.to_string(),
            deleted: true,
        })
    }

    async fn download_file_content(&self, file_id: &str) -> ditto_llm::Result<FileContent> {
        assert_eq!(file_id, "file_fake");
        Ok(FileContent {
            bytes: b"hello world".to_vec(),
            media_type: Some("text/plain".to_string()),
        })
    }
}

fn base_gateway() -> Gateway {
    Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: Vec::new(),
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
    })
}

#[tokio::test]
async fn gateway_translation_files_upload() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let body = format!(
        "--{boundary}\r\n\
Content-Disposition: form-data; name=\"purpose\"\r\n\
\r\n\
fine-tune\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\
Content-Type: text/plain\r\n\
\r\n\
hello world\r\n\
--{boundary}--\r\n"
    );

    let request = Request::builder()
        .method("POST")
        .uri("/v1/files")
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("file_fake"));
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("file"));
    assert_eq!(
        parsed.get("filename").and_then(|v| v.as_str()),
        Some("hello.txt")
    );
    assert_eq!(
        parsed.get("purpose").and_then(|v| v.as_str()),
        Some("fine-tune")
    );
    assert_eq!(parsed.get("bytes").and_then(|v| v.as_u64()), Some(11));
    assert!(parsed.get("created_at").and_then(|v| v.as_u64()).is_some());

    Ok(())
}

#[tokio::test]
async fn gateway_translation_files_list() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/files")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("list"));
    let data = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .expect("data array");
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("id").and_then(|v| v.as_str()),
        Some("file_fake")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_files_retrieve() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/files/file_fake")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("file_fake"));
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("file"));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_files_delete() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("DELETE")
        .uri("/v1/files/file_fake")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("fake")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("file_fake"));
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("file"));
    assert_eq!(parsed.get("deleted").and_then(|v| v.as_bool()), Some(true));

    Ok(())
}

#[tokio::test]
async fn gateway_translation_files_content() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/files/file_fake/content")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("fake")
    );
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"hello world");

    Ok(())
}
