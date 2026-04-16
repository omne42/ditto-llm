use super::*;

#[cfg(feature = "gateway-translation")]
fn translation_model_is_routable(
    state: &GatewayHttpState,
    key: &VirtualKeyConfig,
    request_id: &str,
    request_model_id: &str,
    backend_name: &str,
) -> bool {
    state
        .select_backends_for_model_seeded(request_model_id, Some(key), Some(request_id))
        .is_ok_and(|backends| backends.iter().any(|candidate| candidate == backend_name))
}

pub(super) async fn handle_openai_models_list(
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

    let token = extract_virtual_key(&parts.headers).ok_or_else(|| {
        openai_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            Some("invalid_api_key"),
            "missing virtual key",
        )
    })?;
    let key = state
        .virtual_key_by_token(&token)
        .filter(|key| key.enabled)
        .ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                Some("invalid_api_key"),
                "unauthorized virtual key",
            )
        })?;
    let strip_authorization = true;
    let key_route = key.route.clone();

    let mut base_headers = parts.headers.clone();
    sanitize_proxy_headers(&mut base_headers, strip_authorization);
    insert_request_id(&mut base_headers, &request_id);

    let mut backends: Vec<(String, ProxyBackend)> = state
        .backends
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
            if state.backends.translation_backends.contains_key(route) {
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
                .request_with_timeout(
                    reqwest::Method::GET,
                    "/v1/models",
                    headers,
                    None,
                    Some(timeout),
                )
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
        let bytes =
            match read_reqwest_body_bytes_limited(response, PER_BACKEND_MAX_BODY_BYTES).await {
                Ok(bytes) => Bytes::from(bytes),
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
    let mut translation_backends_in_response = std::collections::BTreeSet::<String>::new();

    #[cfg(feature = "gateway-translation")]
    if !state.backends.translation_backends.is_empty() {
        let mut translation_backend_names = state
            .backends
            .translation_backends
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        translation_backend_names.sort();

        for backend_name in translation_backend_names {
            let Some(backend) = state.backends.translation_backends.get(&backend_name) else {
                continue;
            };
            let models =
                super::translation::collect_models_from_translation_backend(&backend_name, backend);
            for (id, owned_by) in models {
                if !translation_model_is_routable(&state, &key, &request_id, &id, &backend_name) {
                    continue;
                }
                models_by_id.entry(id.to_string()).or_insert_with(|| {
                    translation_backends_in_response.insert(backend_name.clone());
                    super::translation::model_to_openai(&id, &owned_by, created)
                });
            }
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
    let bytes = serde_json::to_vec(&response_json)
        .unwrap_or_else(|_| response_json.to_string().into_bytes());

    let mut response = axum::response::Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    #[cfg(feature = "gateway-translation")]
    if !translation_backends_in_response.is_empty() {
        let translation_header = if translation_backends_in_response.len() == 1 {
            translation_backends_in_response
                .iter()
                .next()
                .and_then(|backend| axum::http::HeaderValue::from_str(backend).ok())
                .unwrap_or_else(|| axum::http::HeaderValue::from_static("multi"))
        } else {
            axum::http::HeaderValue::from_static("multi")
        };
        response
            .headers_mut()
            .insert("x-ditto-translation", translation_header);
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    Ok(response)
}
