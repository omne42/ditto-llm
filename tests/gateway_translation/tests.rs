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
async fn gateway_translation_responses_retrieve_and_input_items() -> ditto_llm::Result<()> {
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
        "input": "hi"
    });
    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);

    let retrieve_request = Request::builder()
        .method("GET")
        .uri("/v1/responses/resp_fake")
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.clone().oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::OK);
    assert_eq!(
        retrieve_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("fake")
    );
    let retrieve_body = to_bytes(retrieve_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let retrieve_parsed: serde_json::Value = serde_json::from_slice(&retrieve_body)?;
    assert_eq!(
        retrieve_parsed.get("id").and_then(|v| v.as_str()),
        Some("resp_fake")
    );
    assert_eq!(
        retrieve_parsed.get("output_text").and_then(|v| v.as_str()),
        Some("hello")
    );

    let input_items_request = Request::builder()
        .method("GET")
        .uri("/v1/responses/resp_fake/input_items")
        .body(Body::empty())
        .unwrap();
    let input_items_response = app.oneshot(input_items_request).await.unwrap();
    assert_eq!(input_items_response.status(), StatusCode::OK);
    let input_items_body = to_bytes(input_items_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let input_items_parsed: serde_json::Value = serde_json::from_slice(&input_items_body)?;
    assert_eq!(
        input_items_parsed.get("object").and_then(|v| v.as_str()),
        Some("list")
    );
    assert_eq!(
        input_items_parsed
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("message")
    );
    assert_eq!(
        input_items_parsed
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str()),
        Some("hi")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_delete() -> ditto_llm::Result<()> {
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
        "input": "hi"
    });
    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);

    let delete_request = Request::builder()
        .method("DELETE")
        .uri("/v1/responses/resp_fake")
        .body(Body::empty())
        .unwrap();
    let delete_response = app.clone().oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);
    let delete_body = to_bytes(delete_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let delete_parsed: serde_json::Value = serde_json::from_slice(&delete_body)?;
    assert_eq!(
        delete_parsed.get("id").and_then(|v| v.as_str()),
        Some("resp_fake")
    );
    assert_eq!(
        delete_parsed.get("deleted").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        delete_parsed.get("object").and_then(|v| v.as_str()),
        Some("response")
    );

    let retrieve_request = Request::builder()
        .method("GET")
        .uri("/v1/responses/resp_fake")
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[cfg(feature = "gateway-tokenizer")]
#[tokio::test]
async fn gateway_translation_responses_input_tokens() -> ditto_llm::Result<()> {
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
        "input": [{"role":"user","content":"count me"}]
    });
    let expected = ditto_llm::gateway::token_count::estimate_input_tokens(
        "/v1/responses",
        "gpt-4o-mini",
        &payload,
    )
    .expect("token estimate");

    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses/input_tokens")
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
        parsed.get("object").and_then(|v| v.as_str()),
        Some("response.input_tokens")
    );
    assert_eq!(
        parsed.get("input_tokens").and_then(|v| v.as_u64()),
        Some(u64::from(expected))
    );

    Ok(())
}

#[cfg(not(feature = "gateway-tokenizer"))]
#[tokio::test]
async fn gateway_translation_responses_input_tokens_requires_tokenizer() -> ditto_llm::Result<()> {
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
        "input": [{"role":"user","content":"count me"}]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses/input_tokens")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("responses input_tokens endpoint requires gateway-tokenizer feature")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_input_tokens_get_not_treated_as_retrieve()
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

    let request = Request::builder()
        .method("GET")
        .uri("/v1/responses/input_tokens")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("unsupported_endpoint")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_retrieve_unknown() -> ditto_llm::Result<()> {
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
        .uri("/v1/responses/resp_missing")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("response resp_missing not found")
    );

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
async fn gateway_translation_rejects_endpoint_without_bound_capability() -> ditto_llm::Result<()> {
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
        "model": "text-embedding-3-small",
        "input": ["a"]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("unsupported_endpoint")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_rejects_model_capability_mismatch_before_builder_resolution()
-> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("openai", Arc::new(FakeModel)).with_provider_config(
            ditto_llm::ProviderConfig {
                base_url: Some("https://api.openai.com/v1".to_string()),
                default_model: Some("gpt-4o-mini".to_string()),
                ..ditto_llm::ProviderConfig::default()
            },
        ),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "input": ["a"]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("unsupported_endpoint")
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
async fn gateway_translation_images_edits_non_streaming() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_model_map(BTreeMap::from([(
                "gpt-image-1".to_string(),
                "image-edit-v2".to_string(),
            )]))
            .with_image_edit_model(Arc::new(FakeImageEditModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-image-1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\nremove background\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"n\"\r\n\r\n2\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"size\"\r\n\r\n1024x1024\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\nb64_json\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"image.png\"\r\nContent-Type: image/png\r\n\r\nimage-bytes\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"mask\"; filename=\"mask.png\"\r\nContent-Type: image/png\r\n\r\nmask-bytes\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
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
    assert!(parsed.get("created").and_then(|v| v.as_u64()).is_some());
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(2)
    );
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str()),
        Some("https://example.com/edited.png")
    );
    assert_eq!(
        parsed
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|v| v.get(1))
            .and_then(|v| v.get("b64_json"))
            .and_then(|v| v.as_str()),
        Some("ZWRpdGVk")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_images_edits_rejects_stream_true() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_image_edit_model(Arc::new(FakeImageEditModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\nremove background\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"stream\"\r\n\r\ntrue\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"image.png\"\r\nContent-Type: image/png\r\n\r\nimage-bytes\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("images endpoint does not support stream=true")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_images_edits_rejects_malformed_multipart() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_image_edit_model(Arc::new(FakeImageEditModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\nremove background\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("images/edits request missing image")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_create_json() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_model_map(BTreeMap::from([(
                "sora-2".to_string(),
                "sora-fast".to_string(),
            )]))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "sora-2",
        "prompt": "road at dusk",
        "seconds": 4,
        "size": "1280x720"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/videos")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("vid_json"));
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("video"));
    assert_eq!(
        parsed.get("model").and_then(|v| v.as_str()),
        Some("sora-fast")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_create_multipart() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_model_map(BTreeMap::from([(
                "sora-2".to_string(),
                "sora-fast".to_string(),
            )]))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let boundary = "ditto_boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nsora-2\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\nremix shot\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"seconds\"\r\n\r\n6\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"size\"\r\n\r\n720p\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"input_reference\"; filename=\"shot.mp4\"\r\nContent-Type: video/mp4\r\n\r\nvideo-bytes\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/videos")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(multipart))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("vid_multipart")
    );
    assert_eq!(parsed.get("seconds").and_then(|v| v.as_str()), Some("6"));
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_list_retrieve_delete() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let list_request = Request::builder()
        .method("GET")
        .uri("/v1/videos?limit=2&after=vid_111&order=desc")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list_request).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_parsed: serde_json::Value = serde_json::from_slice(&list_body)?;
    assert_eq!(
        list_parsed.get("object").and_then(|v| v.as_str()),
        Some("list")
    );
    assert_eq!(
        list_parsed.get("last_id").and_then(|v| v.as_str()),
        Some("vid_123")
    );

    let get_request = Request::builder()
        .method("GET")
        .uri("/v1/videos/vid_123")
        .body(Body::empty())
        .unwrap();
    let get_response = app.clone().oneshot(get_request).await.unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = to_bytes(get_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let get_parsed: serde_json::Value = serde_json::from_slice(&get_body)?;
    assert_eq!(
        get_parsed.get("id").and_then(|v| v.as_str()),
        Some("vid_123")
    );
    assert_eq!(
        get_parsed.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );

    let delete_request = Request::builder()
        .method("DELETE")
        .uri("/v1/videos/vid_123")
        .body(Body::empty())
        .unwrap();
    let delete_response = app.oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);
    let delete_body = to_bytes(delete_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let delete_parsed: serde_json::Value = serde_json::from_slice(&delete_body)?;
    assert_eq!(
        delete_parsed.get("deleted").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        delete_parsed.get("object").and_then(|v| v.as_str()),
        Some("video.deleted")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_content_download() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/videos/vid_123/content?variant=thumbnail")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"thumbnail-bytes");
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_content_rejects_invalid_variant() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/v1/videos/vid_123/content?variant=poster")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("videos content request has unsupported variant: poster")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_remix_json() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "prompt": "change angle"
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/videos/vid_123/remix")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("vid_remix"));
    assert_eq!(
        parsed.get("remixed_from_video_id").and_then(|v| v.as_str()),
        Some("vid_123")
    );
    assert_eq!(
        parsed.get("prompt").and_then(|v| v.as_str()),
        Some("change angle")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_remix_rejects_stream_true() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "prompt": "change angle",
        "stream": true
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/videos/vid_123/remix")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("videos endpoint does not support stream=true")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_rejects_stream_true() -> ditto_llm::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = ditto_llm::gateway::http::router(state);

    let payload = json!({
        "model": "sora-2",
        "prompt": "road at dusk",
        "stream": true
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/videos")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("videos endpoint does not support stream=true")
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
