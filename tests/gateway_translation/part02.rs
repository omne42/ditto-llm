#[tokio::test]
async fn gateway_translation_responses_compact_non_streaming() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeCompactionModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "instructions": "inst",
        "input": [
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}
        ],
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses/compact")
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
        parsed.get("output"),
        Some(&json!([
            {"type":"message","role":"user","content":[{"type":"input_text","text":"compacted"}]}
        ]))
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
async fn gateway_translation_audio_translations_non_streaming() -> ditto_llm::Result<()> {
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
        .uri("/v1/audio/translations")
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

#[cfg(all(feature = "openai-compatible", feature = "embeddings"))]
#[tokio::test]
async fn translation_backend_uses_dotenv_for_lazy_embedding_clients() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = httpmock::MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v1/embeddings")
            .header("authorization", "Bearer sk-dotenv");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "data": [{"embedding": [1.0, 2.0]}]
            }));
    });

    let provider_config = ditto_llm::ProviderConfig {
        base_url: Some(upstream.url("/v1")),
        default_model: Some("gpt-4o-mini".to_string()),
        auth: Some(ditto_llm::ProviderAuth::ApiKeyEnv {
            keys: vec!["DITTO_TEST_DOTENV_API_KEY".to_string()],
        }),
        ..ditto_llm::ProviderConfig::default()
    };
    let env = ditto_llm::Env {
        dotenv: std::collections::BTreeMap::from([(
            "DITTO_TEST_DOTENV_API_KEY".to_string(),
            "sk-dotenv".to_string(),
        )]),
    };

    let model = ditto_llm::gateway::translation::build_language_model(
        "openai-compatible",
        &provider_config,
        &env,
    )
    .await?;
    let backend = TranslationBackend::new("openai-compatible", model)
        .with_env(env)
        .with_provider_config(provider_config);

    let embeddings = backend
        .embed("text-embedding-3-small", vec!["hello".to_string()])
        .await?;
    assert_eq!(embeddings, vec![vec![1.0, 2.0]]);
    mock.assert();
    Ok(())
}

#[cfg(feature = "cohere")]
#[tokio::test]
async fn build_language_model_supports_cohere_from_config() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = httpmock::MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v2/chat")
            .header("authorization", "Bearer sk-dotenv")
            .body_includes("\"model\":\"command-r\"")
            .body_includes("\"role\":\"user\"")
            .body_includes("\"content\":\"hi\"");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "id": "chat_123",
                "finish_reason": "COMPLETE",
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "hello" }
                    ]
                },
                "usage": { "tokens": { "input_tokens": 1, "output_tokens": 2 } }
            }));
    });

    let provider_config = ditto_llm::ProviderConfig {
        base_url: Some(upstream.url("/v2")),
        default_model: Some("command-r".to_string()),
        auth: Some(ditto_llm::ProviderAuth::ApiKeyEnv {
            keys: vec!["DITTO_TEST_DOTENV_COHERE_API_KEY".to_string()],
        }),
        ..ditto_llm::ProviderConfig::default()
    };
    let env = ditto_llm::Env {
        dotenv: std::collections::BTreeMap::from([(
            "DITTO_TEST_DOTENV_COHERE_API_KEY".to_string(),
            "sk-dotenv".to_string(),
        )]),
    };

    let model =
        ditto_llm::gateway::translation::build_language_model("cohere", &provider_config, &env)
            .await?;
    let response = model
        .generate(GenerateRequest::from(vec![Message::user("hi")]))
        .await?;

    assert_eq!(response.text(), "hello");
    assert_eq!(response.usage.input_tokens, Some(1));
    assert_eq!(response.usage.output_tokens, Some(2));

    mock.assert();
    Ok(())
}
