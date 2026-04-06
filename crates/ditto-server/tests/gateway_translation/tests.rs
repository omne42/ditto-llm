fn streamed_response_id_from_sse(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let start = text.find("resp_ditto_").expect("streamed response id");
    text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect()
}

#[tokio::test]
async fn gateway_translation_responses_compact_non_streaming() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeCompactionModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

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
async fn gateway_translation_responses_streaming() -> ditto_core::error::Result<()> {
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
async fn gateway_translation_streamed_responses_can_be_retrieved() -> ditto_core::error::Result<()>
{
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
        "input": [{"role":"user","content":"hi"}]
    });
    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_id = streamed_response_id_from_sse(&create_body);
    assert!(created_id.starts_with("resp_ditto_"));

    let retrieve_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::OK);
    let retrieve_body = to_bytes(retrieve_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let retrieve_parsed: serde_json::Value = serde_json::from_slice(&retrieve_body)?;
    assert_eq!(
        retrieve_parsed.get("id").and_then(|value| value.as_str()),
        Some(created_id.as_str())
    );
    assert_eq!(
        retrieve_parsed
            .get("output_text")
            .and_then(|value| value.as_str()),
        Some("hello")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_streamed_responses_input_items_and_delete()
-> ditto_core::error::Result<()> {
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
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_id = streamed_response_id_from_sse(&create_body);

    let input_items_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}/input_items"))
        .body(Body::empty())
        .unwrap();
    let input_items_response = app.clone().oneshot(input_items_request).await.unwrap();
    assert_eq!(input_items_response.status(), StatusCode::OK);
    assert_eq!(
        input_items_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("primary")
    );
    let input_items_body = to_bytes(input_items_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let input_items_parsed: serde_json::Value = serde_json::from_slice(&input_items_body)?;
    assert_eq!(
        input_items_parsed
            .get("data")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.get("content"))
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.get("text"))
            .and_then(|value| value.as_str()),
        Some("hi")
    );

    let delete_request = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let delete_response = app.clone().oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);
    let delete_body = to_bytes(delete_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let delete_parsed: serde_json::Value = serde_json::from_slice(&delete_body)?;
    assert_eq!(
        delete_parsed.get("id").and_then(|value| value.as_str()),
        Some(created_id.as_str())
    );
    assert_eq!(
        delete_parsed.get("deleted").and_then(|value| value.as_bool()),
        Some(true)
    );

    let retrieve_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_require_gateway_scoped_ids()
-> ditto_core::error::Result<()> {
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

    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "input": "hi"
            })
            .to_string(),
        ))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_id = serde_json::from_slice::<serde_json::Value>(&create_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("response id")
        .to_string();
    let raw_response_id = created_id
        .split_once("_primary_")
        .map(|(_, response_id)| response_id)
        .expect("gateway-scoped response id")
        .to_string();

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{raw_response_id}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str()),
        Some(
            format!(
                "response {raw_response_id} not found; translated response retrieval requires a gateway-scoped id from a /v1/responses create on the same gateway instance and virtual key"
            )
            .as_str(),
        )
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_retrieve_and_input_items() -> ditto_core::error::Result<()> {
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
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let create_parsed: serde_json::Value = serde_json::from_slice(&create_body)?;
    let created_id = create_parsed
        .get("id")
        .and_then(|v| v.as_str())
        .expect("response id")
        .to_string();
    assert!(created_id.starts_with("resp_ditto_"));

    let retrieve_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.clone().oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::OK);
    assert_eq!(
        retrieve_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );
    let retrieve_body = to_bytes(retrieve_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let retrieve_parsed: serde_json::Value = serde_json::from_slice(&retrieve_body)?;
    assert_eq!(
        retrieve_parsed.get("id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );
    assert_eq!(
        retrieve_parsed.get("output_text").and_then(|v| v.as_str()),
        Some("hello")
    );

    let input_items_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}/input_items"))
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
async fn gateway_translation_responses_retrieve_and_delete_are_backend_scoped()
-> ditto_core::error::Result<()> {
    let mut primary_key = ditto_server::gateway::VirtualKeyConfig::new("key-primary", "vk-primary");
    primary_key.route = Some("primary".to_string());
    let mut secondary_key =
        ditto_server::gateway::VirtualKeyConfig::new("key-secondary", "vk-secondary");
    secondary_key.route = Some("secondary".to_string());

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![primary_key, secondary_key],
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

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let payload = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });

    let primary_create = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-primary")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let primary_response = app.clone().oneshot(primary_create).await.unwrap();
    assert_eq!(primary_response.status(), StatusCode::OK);
    let primary_body = to_bytes(primary_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let primary_id = serde_json::from_slice::<serde_json::Value>(&primary_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("primary response id")
        .to_string();

    let secondary_create = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-secondary")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let secondary_response = app.clone().oneshot(secondary_create).await.unwrap();
    assert_eq!(secondary_response.status(), StatusCode::OK);
    let secondary_body = to_bytes(secondary_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let secondary_id = serde_json::from_slice::<serde_json::Value>(&secondary_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("secondary response id")
        .to_string();

    assert_ne!(primary_id, secondary_id);
    assert!(primary_id.contains("_primary_"));
    assert!(secondary_id.contains("_secondary_"));

    let primary_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{primary_id}"))
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let primary_retrieve_response = app.clone().oneshot(primary_retrieve).await.unwrap();
    assert_eq!(primary_retrieve_response.status(), StatusCode::OK);
    assert_eq!(
        primary_retrieve_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("primary")
    );

    let wrong_key_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{primary_id}"))
        .header("authorization", "Bearer vk-secondary")
        .body(Body::empty())
        .unwrap();
    let wrong_key_response = app.clone().oneshot(wrong_key_retrieve).await.unwrap();
    assert_eq!(wrong_key_response.status(), StatusCode::NOT_FOUND);

    let delete_primary = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{primary_id}"))
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let delete_primary_response = app.clone().oneshot(delete_primary).await.unwrap();
    assert_eq!(delete_primary_response.status(), StatusCode::OK);

    let secondary_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{secondary_id}"))
        .header("authorization", "Bearer vk-secondary")
        .body(Body::empty())
        .unwrap();
    let secondary_retrieve_response = app.clone().oneshot(secondary_retrieve).await.unwrap();
    assert_eq!(secondary_retrieve_response.status(), StatusCode::OK);
    assert_eq!(
        secondary_retrieve_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|value| value.to_str().ok()),
        Some("secondary")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_bare_response_id_is_not_scanned_across_backends()
-> ditto_core::error::Result<()> {
    let mut primary_key = ditto_server::gateway::VirtualKeyConfig::new("key-primary", "vk-primary");
    primary_key.route = Some("primary".to_string());
    let mut secondary_key =
        ditto_server::gateway::VirtualKeyConfig::new("key-secondary", "vk-secondary");
    secondary_key.route = Some("secondary".to_string());

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![primary_key, secondary_key],
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

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-primary")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "input": "hi"
            })
            .to_string(),
        ))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);

    let legacy_retrieve = Request::builder()
        .method("GET")
        .uri("/v1/responses/resp_fake")
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let legacy_retrieve_response = app.clone().oneshot(legacy_retrieve).await.unwrap();
    assert_eq!(legacy_retrieve_response.status(), StatusCode::NOT_FOUND);

    let legacy_delete = Request::builder()
        .method("DELETE")
        .uri("/v1/responses/resp_fake")
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let legacy_delete_response = app.oneshot(legacy_delete).await.unwrap();
    assert_eq!(legacy_delete_response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_retrieve_and_delete_are_virtual_key_scoped_even_on_same_backend()
-> ditto_core::error::Result<()> {
    let mut primary_key = ditto_server::gateway::VirtualKeyConfig::new("key-primary", "vk-primary");
    primary_key.route = Some("primary".to_string());
    let mut secondary_key =
        ditto_server::gateway::VirtualKeyConfig::new("key-secondary", "vk-secondary");
    secondary_key.route = Some("primary".to_string());

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![primary_key, secondary_key],
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

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-primary")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "input": "hi"
            })
            .to_string(),
        ))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_id = serde_json::from_slice::<serde_json::Value>(&create_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("response id")
        .to_string();
    assert!(response_id.contains("_primary_"));

    let secondary_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{response_id}"))
        .header("authorization", "Bearer vk-secondary")
        .body(Body::empty())
        .unwrap();
    let secondary_retrieve_response = app.clone().oneshot(secondary_retrieve).await.unwrap();
    assert_eq!(secondary_retrieve_response.status(), StatusCode::NOT_FOUND);
    let secondary_retrieve_body =
        to_bytes(secondary_retrieve_response.into_body(), usize::MAX)
            .await
            .unwrap();
    let secondary_retrieve_json: serde_json::Value =
        serde_json::from_slice(&secondary_retrieve_body)?;
    assert_eq!(
        secondary_retrieve_json["error"]["message"].as_str(),
        Some(
            format!(
                "response {response_id} not found; translated response retrieval requires a gateway-scoped id from a /v1/responses create on the same gateway instance and virtual key"
            )
            .as_str()
        )
    );

    let secondary_delete = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{response_id}"))
        .header("authorization", "Bearer vk-secondary")
        .body(Body::empty())
        .unwrap();
    let secondary_delete_response = app.clone().oneshot(secondary_delete).await.unwrap();
    assert_eq!(secondary_delete_response.status(), StatusCode::NOT_FOUND);

    let owner_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{response_id}"))
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let owner_retrieve_response = app.clone().oneshot(owner_retrieve).await.unwrap();
    assert_eq!(owner_retrieve_response.status(), StatusCode::OK);

    let owner_delete = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{response_id}"))
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let owner_delete_response = app.oneshot(owner_delete).await.unwrap();
    assert_eq!(owner_delete_response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_retrieve_and_delete_are_virtual_key_scoped_within_backend()
-> ditto_core::error::Result<()> {
    let mut first_key = ditto_server::gateway::VirtualKeyConfig::new("key-first", "vk-first");
    first_key.route = Some("primary".to_string());
    let mut second_key = ditto_server::gateway::VirtualKeyConfig::new("key-second", "vk-second");
    second_key.route = Some("primary".to_string());

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![first_key, second_key],
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

    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);
    let payload = json!({
        "model": "gpt-4o-mini",
        "input": "hi"
    });

    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-first")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_id = serde_json::from_slice::<serde_json::Value>(&create_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("response id")
        .to_string();
    assert!(created_id.contains("_primary_"));

    let wrong_key_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-second")
        .body(Body::empty())
        .unwrap();
    let wrong_key_retrieve_response = app.clone().oneshot(wrong_key_retrieve).await.unwrap();
    assert_eq!(wrong_key_retrieve_response.status(), StatusCode::NOT_FOUND);

    let wrong_key_delete = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-second")
        .body(Body::empty())
        .unwrap();
    let wrong_key_delete_response = app.clone().oneshot(wrong_key_delete).await.unwrap();
    assert_eq!(wrong_key_delete_response.status(), StatusCode::NOT_FOUND);

    let owner_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-first")
        .body(Body::empty())
        .unwrap();
    let owner_retrieve_response = app.clone().oneshot(owner_retrieve).await.unwrap();
    assert_eq!(owner_retrieve_response.status(), StatusCode::OK);

    let owner_delete = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-first")
        .body(Body::empty())
        .unwrap();
    let owner_delete_response = app.clone().oneshot(owner_delete).await.unwrap();
    assert_eq!(owner_delete_response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_delete() -> ditto_core::error::Result<()> {
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
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let create_parsed: serde_json::Value = serde_json::from_slice(&create_body)?;
    let created_id = create_parsed
        .get("id")
        .and_then(|v| v.as_str())
        .expect("response id")
        .to_string();
    assert!(created_id.starts_with("resp_ditto_"));

    let delete_request = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let delete_response = app.clone().oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);
    assert_eq!(
        delete_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );
    let delete_body = to_bytes(delete_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let delete_parsed: serde_json::Value = serde_json::from_slice(&delete_body)?;
    assert_eq!(
        delete_parsed.get("id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
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
        .uri(format!("/v1/responses/{created_id}"))
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_owned_resource_ids_are_scoped_and_owner_bound(
) -> ditto_core::error::Result<()> {
    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![
            ditto_server::gateway::VirtualKeyConfig::new("key-1", "vk-1"),
            ditto_server::gateway::VirtualKeyConfig::new("key-2", "vk-2"),
        ],
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
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_file_client(Arc::new(FakeFileClient))
            .with_batch_client(Arc::new(FakeBatchClient))
            .with_video_generation_model(Arc::new(FakeVideoGenerationModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let file_id = create_owned_translation_file(app.clone()).await?;
    assert!(file_id.starts_with("file_ditto_"));

    let list_files_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_files_response.status(), StatusCode::OK);
    let list_files_body = to_bytes(list_files_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_files_json: serde_json::Value = serde_json::from_slice(&list_files_body)?;
    assert_eq!(
        list_files_json
            .get("data")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|value| value.as_str()),
        Some(file_id.as_str())
    );

    let raw_file_request = Request::builder()
        .method("GET")
        .uri("/v1/files/file_fake")
        .body(Body::empty())
        .unwrap();
    let raw_file_response = app.clone().oneshot(raw_file_request).await.unwrap();
    assert_eq!(raw_file_response.status(), StatusCode::NOT_FOUND);

    let file_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/files/{file_id}"))
        .body(Body::empty())
        .unwrap();
    let file_response = app.clone().oneshot(file_request).await.unwrap();
    assert_eq!(file_response.status(), StatusCode::OK);
    let file_body = to_bytes(file_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let file_json: serde_json::Value = serde_json::from_slice(&file_body)?;
    assert_eq!(file_json.get("id").and_then(|value| value.as_str()), Some(file_id.as_str()));

    let foreign_file_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/files/{file_id}"))
        .header(axum::http::header::AUTHORIZATION, "Bearer vk-2")
        .body(Body::empty())
        .unwrap();
    let foreign_file_response = app.clone().oneshot(foreign_file_request).await.unwrap();
    assert_eq!(foreign_file_response.status(), StatusCode::NOT_FOUND);

    let batch_id = create_owned_translation_batch(app.clone()).await?;
    assert!(batch_id.starts_with("batch_ditto_"));

    let batch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/batches/{batch_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_response.status(), StatusCode::OK);
    let batch_body = to_bytes(batch_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&batch_body)?;
    assert_eq!(
        batch_json.get("id").and_then(|value| value.as_str()),
        Some(batch_id.as_str())
    );

    let batch_list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/batches?after={batch_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_list_response.status(), StatusCode::OK);
    let batch_list_body = to_bytes(batch_list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let batch_list_json: serde_json::Value = serde_json::from_slice(&batch_list_body)?;
    let batch_item = batch_list_json
        .get("data")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .expect("batch item");
    assert_eq!(batch_item.get("id").and_then(|value| value.as_str()), Some(batch_id.as_str()));
    assert_eq!(
        batch_item
            .get("input_file_id")
            .and_then(|value| value.as_str()),
        Some(file_id.as_str())
    );

    let video_id = create_owned_translation_video(app.clone()).await?;
    assert!(video_id.starts_with("video_ditto_"));

    let video_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/videos/{video_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(video_response.status(), StatusCode::OK);
    let video_body = to_bytes(video_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let video_json: serde_json::Value = serde_json::from_slice(&video_body)?;
    assert_eq!(
        video_json.get("id").and_then(|value| value.as_str()),
        Some(video_id.as_str())
    );

    let remix_payload = json!({ "prompt": "change angle" });
    let remix_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/videos/{video_id}/remix"))
                .header("content-type", "application/json")
                .body(Body::from(remix_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(remix_response.status(), StatusCode::OK);
    let remix_body = to_bytes(remix_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let remix_json: serde_json::Value = serde_json::from_slice(&remix_body)?;
    assert!(
        remix_json
            .get("id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.starts_with("video_ditto_"))
    );
    assert_eq!(
        remix_json
            .get("remixed_from_video_id")
            .and_then(|value| value.as_str()),
        Some(video_id.as_str())
    );

    let video_content_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/videos/{video_id}/content?variant=thumbnail"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(video_content_response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_scoped_ids_isolate_same_provider_backends()
-> ditto_core::error::Result<()> {
    let mut primary_key = ditto_server::gateway::VirtualKeyConfig::new("key-primary", "vk-primary");
    primary_key.route = Some("primary".to_string());
    let mut secondary_key =
        ditto_server::gateway::VirtualKeyConfig::new("key-secondary", "vk-secondary");
    secondary_key.route = Some("secondary".to_string());

    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![primary_key, secondary_key],
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
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );
    translation_backends.insert(
        "secondary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let create_request = |token: &str| {
        Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "model": "gpt-4o-mini",
                    "input": "hi"
                })
                .to_string(),
            ))
            .unwrap()
    };

    let primary_create = app.clone().oneshot(create_request("vk-primary")).await.unwrap();
    let primary_body = to_bytes(primary_create.into_body(), usize::MAX)
        .await
        .unwrap();
    let primary_id = serde_json::from_slice::<serde_json::Value>(&primary_body)?
        .get("id")
        .and_then(|v| v.as_str())
        .expect("primary response id")
        .to_string();

    let secondary_create = app
        .clone()
        .oneshot(create_request("vk-secondary"))
        .await
        .unwrap();
    let secondary_body = to_bytes(secondary_create.into_body(), usize::MAX)
        .await
        .unwrap();
    let secondary_id = serde_json::from_slice::<serde_json::Value>(&secondary_body)?
        .get("id")
        .and_then(|v| v.as_str())
        .expect("secondary response id")
        .to_string();

    assert_ne!(primary_id, secondary_id);
    assert!(primary_id.contains("_primary_"));
    assert!(secondary_id.contains("_secondary_"));

    let retrieve = |id: &str, token: &str| {
        Request::builder()
            .method("GET")
            .uri(format!("/v1/responses/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };

    let primary_retrieve = app
        .clone()
        .oneshot(retrieve(&primary_id, "vk-primary"))
        .await
        .unwrap();
    assert_eq!(primary_retrieve.status(), StatusCode::OK);
    assert_eq!(
        primary_retrieve
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );

    let secondary_retrieve = app
        .clone()
        .oneshot(retrieve(&secondary_id, "vk-secondary"))
        .await
        .unwrap();
    assert_eq!(secondary_retrieve.status(), StatusCode::OK);
    assert_eq!(
        secondary_retrieve
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("secondary")
    );

    let delete_primary = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{primary_id}"))
        .header("authorization", "Bearer vk-primary")
        .body(Body::empty())
        .unwrap();
    let delete_primary_response = app.clone().oneshot(delete_primary).await.unwrap();
    assert_eq!(delete_primary_response.status(), StatusCode::OK);
    assert_eq!(
        delete_primary_response
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("primary")
    );

    let primary_missing = app
        .clone()
        .oneshot(retrieve(&primary_id, "vk-primary"))
        .await
        .unwrap();
    assert_eq!(primary_missing.status(), StatusCode::NOT_FOUND);

    let secondary_still_present = app
        .oneshot(retrieve(&secondary_id, "vk-secondary"))
        .await
        .unwrap();
    assert_eq!(secondary_still_present.status(), StatusCode::OK);
    assert_eq!(
        secondary_still_present
            .headers()
            .get("x-ditto-translation")
            .and_then(|v| v.to_str().ok()),
        Some("secondary")
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_responses_reject_other_virtual_keys()
-> ditto_core::error::Result<()> {
    let gateway = Gateway::new(GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![
            ditto_server::gateway::VirtualKeyConfig::new("key-1", "vk-1"),
            ditto_server::gateway::VirtualKeyConfig::new("key-2", "vk-2"),
        ],
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

    let create_request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "input": "hi"
            })
            .to_string(),
        ))
        .unwrap();
    let create_response = app.clone().oneshot(create_request).await.unwrap();
    let create_body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_id = serde_json::from_slice::<serde_json::Value>(&create_body)?
        .get("id")
        .and_then(|value| value.as_str())
        .expect("response id")
        .to_string();

    let retrieve_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-2")
        .body(Body::empty())
        .unwrap();
    let retrieve_response = app.clone().oneshot(retrieve_request).await.unwrap();
    assert_eq!(retrieve_response.status(), StatusCode::NOT_FOUND);

    let delete_request = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-2")
        .body(Body::empty())
        .unwrap();
    let delete_response = app.clone().oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::NOT_FOUND);

    let creator_retrieve = Request::builder()
        .method("GET")
        .uri(format!("/v1/responses/{created_id}"))
        .header("authorization", "Bearer vk-1")
        .body(Body::empty())
        .unwrap();
    let creator_response = app.oneshot(creator_retrieve).await.unwrap();
    assert_eq!(creator_response.status(), StatusCode::OK);

    Ok(())
}

#[cfg(feature = "gateway-tokenizer")]
#[tokio::test]
async fn gateway_translation_responses_input_tokens() -> ditto_core::error::Result<()> {
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

    let payload = json!({
        "model": "gpt-4o-mini",
        "input": [{"role":"user","content":"count me"}]
    });
    let expected = ditto_server::gateway::token_count::estimate_input_tokens(
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
async fn gateway_translation_responses_input_tokens_requires_tokenizer() -> ditto_core::error::Result<()> {
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
-> ditto_core::error::Result<()> {
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
async fn gateway_translation_responses_retrieve_unknown() -> ditto_core::error::Result<()> {
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
        Some(
            "response resp_missing not found; translated response retrieval requires a gateway-scoped id from a /v1/responses create on the same gateway instance and virtual key",
        )
    );

    Ok(())
}

#[tokio::test]
async fn gateway_translation_embeddings_non_streaming() -> ditto_core::error::Result<()> {
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
async fn gateway_translation_rejects_endpoint_without_bound_capability() -> ditto_core::error::Result<()> {
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
-> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("openai", Arc::new(FakeModel)).with_provider_config(
            ditto_core::config::ProviderConfig {
                base_url: Some("https://api.openai.com/v1".to_string()),
                default_model: Some("gpt-4o-mini".to_string()),
                ..ditto_core::config::ProviderConfig::default()
            },
        ),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

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
async fn gateway_translation_moderations_non_streaming() -> ditto_core::error::Result<()> {
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
async fn gateway_translation_spends_tenant_budget_scope() -> ditto_core::error::Result<()> {
    let mut key = ditto_server::gateway::VirtualKeyConfig::new("key-1", "vk-1");
    key.tenant_id = Some("tenant-1".to_string());
    let payload = json!({
        "model": "gpt-4o-mini",
        "max_output_tokens": 0,
        "input": "hi"
    });
    let payload_text = payload.to_string();
    #[cfg(feature = "gateway-tokenizer")]
    let charge_tokens = ditto_server::gateway::token_count::estimate_input_tokens(
        "/v1/responses",
        "gpt-4o-mini",
        &payload,
    )
    .unwrap_or(((payload_text.len().saturating_add(3)) / 4) as u32) as u64;
    #[cfg(not(feature = "gateway-tokenizer"))]
    let charge_tokens = ((payload_text.len().saturating_add(3)) / 4) as u64;
    key.tenant_budget = Some(ditto_server::gateway::BudgetConfig {
        total_tokens: Some(charge_tokens + 1),
        ..ditto_server::gateway::BudgetConfig::default()
    });
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
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);

    let request = || {
        Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("content-type", "application/json")
            .body(Body::from(
                payload_text.clone(),
            ))
            .unwrap()
    };

    let first = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let _ = to_bytes(first.into_body(), usize::MAX).await.unwrap();

    let second = app.oneshot(request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::PAYMENT_REQUIRED);

    Ok(())
}

#[tokio::test]
async fn gateway_translation_images_generations_non_streaming() -> ditto_core::error::Result<()> {
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
async fn gateway_translation_images_edits_non_streaming() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_images_edits_rejects_stream_true() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_images_edits_rejects_malformed_multipart() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_videos_create_json() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("video_ditto_7_primary_vid_json")
    );
    assert_eq!(parsed.get("object").and_then(|v| v.as_str()), Some("video"));
    assert_eq!(
        parsed.get("model").and_then(|v| v.as_str()),
        Some("sora-fast")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_create_multipart() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
        Some("video_ditto_7_primary_vid_multipart")
    );
    assert_eq!(parsed.get("seconds").and_then(|v| v.as_str()), Some("6"));
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_list_retrieve_delete() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_video(app.clone()).await?;

    let list_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos?limit=2&after={created_id}&order=desc"))
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
        list_parsed
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );

    let get_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{created_id}"))
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
        Some(created_id.as_str())
    );
    assert_eq!(
        get_parsed.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );

    let delete_request = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/videos/{created_id}"))
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
async fn gateway_translation_videos_content_download() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_video(app.clone()).await?;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{created_id}/content?variant=thumbnail"))
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
async fn gateway_translation_videos_content_rejects_invalid_variant() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_video(app.clone()).await?;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{created_id}/content?variant=poster"))
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
async fn gateway_translation_videos_remix_json() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_video(app.clone()).await?;

    let payload = json!({
        "prompt": "change angle"
    });
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/videos/{created_id}/remix"))
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some("video_ditto_7_primary_vid_remix")
    );
    assert_eq!(
        parsed.get("remixed_from_video_id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );
    assert_eq!(
        parsed.get("prompt").and_then(|v| v.as_str()),
        Some("change angle")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_videos_remix_rejects_stream_true() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_videos_rejects_stream_true() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_audio_transcriptions_non_streaming() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_audio_translations_non_streaming() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_audio_speech_non_streaming() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn gateway_translation_batches_create() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);
    let file_id = create_owned_translation_file(app.clone()).await?;

    let payload = json!({
        "input_file_id": file_id,
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
        Some("batch_ditto_7_primary_batch_created")
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_list() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_batch(app.clone()).await?;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/batches?limit=2&after={created_id}"))
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
        Some(1)
    );
    assert_eq!(
        parsed.get("last_id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_retrieve() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_batch(app.clone()).await?;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/batches/{created_id}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_batches_cancel() -> ditto_core::error::Result<()> {
    let gateway = base_gateway();
    let mut translation_backends = HashMap::new();
    translation_backends.insert(
        "primary".to_string(),
        TranslationBackend::new("fake", Arc::new(FakeModel))
            .with_batch_client(Arc::new(FakeBatchClient))
            .with_file_client(Arc::new(FakeFileClient)),
    );

    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(HashMap::new())
        .with_translation_backends(translation_backends);
    let app = authorized_test_app(state);
    let created_id = create_owned_translation_batch(app.clone()).await?;

    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/batches/{created_id}/cancel"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_str()),
        Some(created_id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn gateway_translation_rerank_non_streaming() -> ditto_core::error::Result<()> {
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
    let app = authorized_test_app(state);

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
async fn translation_backend_uses_dotenv_for_lazy_embedding_clients() -> ditto_core::error::Result<()> {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = httpmock::MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v1/embeddings")
            .header("authorization", "Bearer sk-dotenv")
            .body_includes(r#""model":"text-embedding-3-small""#)
            .body_includes(r#""input":["hello"]"#);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "data": [
                    { "embedding": [1.0, 2.0] }
                ]
            }));
    });

    let provider_config = ditto_core::config::ProviderConfig {
        base_url: Some(upstream.url("/v1")),
        default_model: Some("gpt-4o-mini".to_string()),
        auth: Some(ditto_core::config::ProviderAuth::ApiKeyEnv {
            keys: vec!["DITTO_TEST_DOTENV_API_KEY".to_string()],
        }),
        ..ditto_core::config::ProviderConfig::default()
    };
    let env = ditto_core::config::Env {
        dotenv: std::collections::BTreeMap::from([(
            "DITTO_TEST_DOTENV_API_KEY".to_string(),
            "sk-dotenv".to_string(),
        )]),
    };

    let model = ditto_core::runtime::build_language_model(
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
async fn build_language_model_supports_cohere_from_config() -> ditto_core::error::Result<()> {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let upstream = httpmock::MockServer::start();
    let mock = upstream.mock(|when, then| {
        // Request-shape coverage lives in the provider unit tests; this integration only verifies
        // that gateway translation can build and execute a Cohere-backed language model.
        when.method(httpmock::Method::POST);
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

    let provider_config = ditto_core::config::ProviderConfig {
        base_url: Some(upstream.url("/v2")),
        default_model: Some("command-r".to_string()),
        auth: Some(ditto_core::config::ProviderAuth::ApiKeyEnv {
            keys: vec!["DITTO_TEST_DOTENV_COHERE_API_KEY".to_string()],
        }),
        ..ditto_core::config::ProviderConfig::default()
    };
    let env = ditto_core::config::Env {
        dotenv: std::collections::BTreeMap::from([(
            "DITTO_TEST_DOTENV_COHERE_API_KEY".to_string(),
            "sk-dotenv".to_string(),
        )]),
    };

    let model =
        ditto_core::runtime::build_language_model("cohere", &provider_config, &env)
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
