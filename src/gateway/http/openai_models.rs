async fn handle_openai_models_list(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    const PER_BACKEND_TIMEOUT_SECS: u64 = 10;
    const PER_BACKEND_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

    let (parts, _body) = req.into_parts();

    let request_id =
        extract_header(&parts.headers, "x-request-id").unwrap_or_else(generate_request_id);
    #[cfg(feature = "gateway-translation")]
    let created = now_epoch_seconds();

    let (strip_authorization, key_route) = {
        let gateway = state.gateway.lock().await;
        if gateway.config.virtual_keys.is_empty() {
            (false, None)
        } else {
            let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
                openai_error(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    Some("invalid_api_key"),
                    "missing virtual key",
                )
            })?;
            let key = gateway
                .virtual_key_by_token(&token)
                .filter(|key| key.enabled)
                .cloned()
                .ok_or_else(|| {
                    openai_error(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        Some("invalid_api_key"),
                        "unauthorized virtual key",
                    )
                })?;
            (true, key.route)
        }
    };

    let mut base_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut base_headers, strip_authorization);
    insert_request_id(&mut base_headers, &request_id);

    let mut backends: Vec<(String, ProxyBackend)> = state
        .proxy_backends
        .iter()
        .map(|(name, backend)| (name.clone(), backend.clone()))
        .collect();
    backends.sort_by(|(a, _), (b, _)| a.cmp(b));

    if let Some(route) = key_route.as_deref() {
        if let Some((name, backend)) = backends.into_iter().find(|(name, _)| name == route) {
            backends = vec![(name, backend)];
        } else {
            #[cfg(feature = "gateway-translation")]
            if state.translation_backends.contains_key(route) {
                backends = Vec::new();
            } else {
                return Err(openai_error(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    Some("backend_not_found"),
                    format!("backend not found: {route}"),
                ));
            }

            #[cfg(not(feature = "gateway-translation"))]
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                format!("backend not found: {route}"),
            ));
        }
    }

    let had_proxy_backends = !backends.is_empty();
    let results = futures_util::future::join_all(backends.into_iter().map(|(name, backend)| {
        let mut headers = base_headers.clone();
        apply_backend_headers(&mut headers, backend.headers());
        let timeout = std::time::Duration::from_secs(PER_BACKEND_TIMEOUT_SECS);
        async move {
            let response = backend
                .request_with_timeout(reqwest::Method::GET, "/v1/models", headers, None, Some(timeout))
                .await;
            (name, response)
        }
    }))
    .await;

    let mut models_by_id: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    for (backend_name, result) in results {
        let response = match result {
            Ok(response) => response,
            Err(_) => continue,
        };
        if !response.status().is_success() {
            continue;
        }
        let headers = response.headers().clone();
        let bytes = match read_reqwest_body_bytes_bounded_with_content_length(
            response,
            &headers,
            PER_BACKEND_MAX_BODY_BYTES,
        )
        .await
        {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let models = json
            .get("data")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for model in models {
            let Some(id) = model.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            models_by_id.entry(id.to_string()).or_insert(model);
        }
        emit_json_log(
            &state,
            "models.backend_ok",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "models": models_by_id.len(),
            }),
        );
    }

    #[cfg(feature = "gateway-translation")]
    let has_translation_backends = !state.translation_backends.is_empty();

    #[cfg(feature = "gateway-translation")]
    if has_translation_backends {
        let models =
            super::translation::collect_models_from_translation_backends(state.translation_backends.as_ref());
        for (id, owned_by) in models {
            models_by_id
                .entry(id.to_string())
                .or_insert_with(|| super::translation::model_to_openai(&id, &owned_by, created));
        }
    }

    if models_by_id.is_empty() {
        if !had_proxy_backends {
            return Err(openai_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                Some("backend_not_found"),
                "no proxy backends configured",
            ));
        }
        return Err(openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            "all upstream /v1/models requests failed",
        ));
    }

    let response_json = serde_json::json!({
        "object": "list",
        "data": models_by_id.into_values().collect::<Vec<_>>(),
    });
    let bytes =
        serde_json::to_vec(&response_json).unwrap_or_else(|_| response_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
    #[cfg(feature = "gateway-translation")]
    if has_translation_backends {
        response
            .headers_mut()
            .insert(
                "x-ditto-translation",
                axum::http::HeaderValue::from_static("multi"),
            );
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    Ok(response)
}
