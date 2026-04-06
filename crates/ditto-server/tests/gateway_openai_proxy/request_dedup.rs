#[cfg(feature = "gateway-store-sqlite")]
fn dedup_test_request(request_id: &str, input: &str) -> Request<Body> {
    dedup_test_request_with_header(request_id, input, None)
}

#[cfg(feature = "gateway-store-sqlite")]
fn dedup_test_request_with_header(
    request_id: &str,
    input: &str,
    header: Option<(&str, &str)>,
) -> Request<Body> {
    let body = json!({
        "model": "gpt-4o-mini",
        "input": input,
    });
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", request_id)
        .header("content-type", "application/json");
    if let Some((name, value)) = header {
        builder = builder.header(name, value);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

#[cfg(feature = "gateway-store-sqlite")]
fn dedup_test_stream_request(request_id: &str, input: &str) -> Request<Body> {
    let body = json!({
        "model": "gpt-4o-mini",
        "input": input,
        "stream": true,
    });
    Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", "Bearer vk-1")
        .header("x-request-id", request_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[cfg(feature = "gateway-store-sqlite")]
async fn start_chunked_upstream(
    content_type: &'static str,
    chunks: &'static [&'static [u8]],
) -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use bytes::Bytes;

    #[derive(Clone)]
    struct ChunkedUpstreamState {
        hits: Arc<AtomicUsize>,
        content_type: &'static str,
        chunks: &'static [&'static [u8]],
    }

    async fn handler(
        State(state): State<ChunkedUpstreamState>,
        req: Request<Body>,
    ) -> impl IntoResponse {
        assert_eq!(
            req.headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer sk-test")
        );
        state.hits.fetch_add(1, Ordering::SeqCst);

        let stream = futures_util::stream::iter(state.chunks.iter().copied().map(|chunk| {
            Ok::<Bytes, Infallible>(Bytes::copy_from_slice(chunk))
        }));
        axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", state.content_type)
            .body(Body::from_stream(stream))
            .expect("chunked response")
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind chunked upstream");
    let addr = listener.local_addr().expect("chunked upstream addr");
    let app = axum::Router::new()
        .route("/v1/responses", post(handler))
        .with_state(ChunkedUpstreamState {
            hits: hits.clone(),
            content_type,
            chunks,
        });
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve chunked upstream");
    });

    (format!("http://{addr}"), hits, server)
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_replays_completed_request_by_client_request_id() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"resp-1"}"#);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_request("req-dedup-1", "hi"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        first
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|v| v.to_str().ok()),
        Some("leader")
    );
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, r#"{"id":"resp-1"}"#);

    let second = app
        .oneshot(dedup_test_request("req-dedup-1", "hi"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|v| v.to_str().ok()),
        Some("replay")
    );
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    assert_eq!(second_body, r#"{"id":"resp-1"}"#);

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_replays_streamed_request_by_client_request_id() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-stream-1");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body("data: first\n\ndata: second\n\n");
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_stream_request("req-dedup-stream-1", "hi"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        first
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(
        first
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|v| v.to_str().ok()),
        Some("leader")
    );
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, "data: first\n\ndata: second\n\n");

    let second = app
        .oneshot(dedup_test_stream_request("req-dedup-stream-1", "hi"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(
        second
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|v| v.to_str().ok()),
        Some("replay")
    );
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    assert_eq!(second_body, "data: first\n\ndata: second\n\n");

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_coalesces_in_flight_duplicates_by_client_request_id() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-2");
        then.status(200)
            .header("content-type", "application/json")
            .delay(std::time::Duration::from_millis(200))
            .body(r#"{"id":"resp-2"}"#);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let (first, second) = tokio::join!(
        app.clone().oneshot(dedup_test_request("req-dedup-2", "hi")),
        app.clone().oneshot(dedup_test_request("req-dedup-2", "hi")),
    );
    let first = first.unwrap();
    let second = second.unwrap();

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first_kind = first
        .headers()
        .get("x-ditto-request-dedup")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let second_kind = second
        .headers()
        .get("x-ditto-request-dedup")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(matches!(first_kind.as_str(), "leader" | "replay"));
    assert!(matches!(second_kind.as_str(), "leader" | "replay"));
    assert_ne!(first_kind, second_kind);

    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, r#"{"id":"resp-2"}"#);
    assert_eq!(second_body, r#"{"id":"resp-2"}"#);

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_rejects_conflicting_request_reuse_by_client_request_id() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-3");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"resp-3"}"#);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_request("req-dedup-3", "hi"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, r#"{"id":"resp-3"}"#);

    let second = app
        .oneshot(dedup_test_request("req-dedup-3", "different"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("request_id_conflict")
    );

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_rejects_request_reuse_when_upstream_headers_change() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-header-conflict")
            .header("openai-organization", "org-a");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"resp-header-conflict"}"#);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_request_with_header(
            "req-dedup-header-conflict",
            "hi",
            Some(("openai-organization", "org-a")),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(dedup_test_request_with_header(
            "req-dedup-header-conflict",
            "hi",
            Some(("openai-organization", "org-b")),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("request_id_conflict")
    );

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_replays_when_only_trace_headers_change() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-trace-only");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"resp-trace-only"}"#);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_request_with_header(
            "req-dedup-trace-only",
            "hi",
            Some((
                "traceparent",
                "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
            )),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(dedup_test_request_with_header(
            "req-dedup-trace-only",
            "hi",
            Some((
                "traceparent",
                "00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01",
            )),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|value| value.to_str().ok()),
        Some("replay")
    );

    mock.assert_calls(1);
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_replays_chunked_non_sse_request_without_content_length() {
    use std::sync::atomic::Ordering;

    let (base_url, hits, upstream) = start_chunked_upstream(
        "application/json",
        &[br#"{"id":"resp-"#, br#"chunked"}"#],
    )
    .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config("primary", base_url, "Bearer sk-test")],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_request("req-dedup-chunked-json", "hi"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        first
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()),
        None
    );
    assert_eq!(
        first
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|value| value.to_str().ok()),
        Some("leader")
    );
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body.as_ref(), br#"{"id":"resp-chunked"}"#);

    let second = app
        .oneshot(dedup_test_request("req-dedup-chunked-json", "hi"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second
            .headers()
            .get("x-ditto-request-dedup")
            .and_then(|value| value.to_str().ok()),
        Some("replay")
    );
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    assert_eq!(second_body.as_ref(), br#"{"id":"resp-chunked"}"#);

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    upstream.abort();
}

#[cfg(feature = "gateway-store-sqlite")]
#[tokio::test]
async fn openai_compat_proxy_marks_large_stream_replay_unavailable_by_client_request_id() {
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return;
    }

    let upstream = MockServer::start();
    let large_stream = concat!(
        "data: 0123456789abcdef0123456789abcdef0123456789abcdef",
        "0123456789abcdef0123456789abcdef0123456789abcdef",
        "0123456789abcdef0123456789abcdef0123456789abcdef",
        "0123456789abcdef0123456789abcdef0123456789abcdef\n\n"
    );
    let mock = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/responses")
            .header("authorization", "Bearer sk-test")
            .header("x-request-id", "req-dedup-large-stream");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(large_stream);
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("gateway.sqlite");
    let store = ditto_server::gateway::SqliteStore::new(&db_path);
    store.init().await.expect("init sqlite");

    let config = GatewayConfig {
        backends: vec![backend_config(
            "primary",
            upstream.base_url(),
            "Bearer sk-test",
        )],
        virtual_keys: vec![VirtualKeyConfig::new("key-1", "vk-1")],
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
    };
    let proxy_backends = build_proxy_backends(&config).expect("proxy backends");
    let gateway = Gateway::new(config);
    let state = GatewayHttpState::new(gateway)
        .with_proxy_backends(proxy_backends)
        .with_sqlite_store(store)
        .with_proxy_max_body_bytes(128);
    let app = ditto_server::gateway::http::router(state);

    let first = app
        .clone()
        .oneshot(dedup_test_stream_request("req-dedup-large-stream", "hi"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, large_stream);

    let second = app
        .oneshot(dedup_test_stream_request("req-dedup-large-stream", "hi"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(
        parsed
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str()),
        Some("request_id_replay_unavailable")
    );

    mock.assert_calls(1);
}
