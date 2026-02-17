#[cfg(feature = "gateway-translation")]
async fn attempt_translation_backend(
    params: ProxyAttemptParams<'_>,
    backend_name: &str,
    translation_backend: super::TranslationBackend,
    attempted_backends: &[String],
) -> Result<BackendAttemptOutcome, (StatusCode, Json<OpenAiErrorResponse>)> {
    let state = params.state;
    let parts = params.parts;
    let body = params.body;
    let parsed_json = params.parsed_json;
    let model = params.model;
    #[cfg(feature = "gateway-costing")]
    let service_tier = params.service_tier;
    #[cfg(not(feature = "gateway-costing"))]
    let _service_tier = params.service_tier;
    let request_id = params.request_id;
    let path_and_query = params.path_and_query;
    let _now_epoch_seconds = params.now_epoch_seconds;
    let charge_tokens = params.charge_tokens;
    let _stream_requested = params.stream_requested;
    let use_persistent_budget = params.use_persistent_budget;
    #[cfg(not(feature = "gateway-costing"))]
    let _ = use_persistent_budget;
    let virtual_key_id = params.virtual_key_id;
    let budget = params.budget;
    let project_budget_scope = params.project_budget_scope;
    let user_budget_scope = params.user_budget_scope;
    let charge_cost_usd_micros = params.charge_cost_usd_micros;

    #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    let _cost_budget_reserved = params.cost_budget_reserved;
    #[cfg(all(
        feature = "gateway-costing",
        any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
    ))]
    let cost_budget_reservation_ids = params.cost_budget_reservation_ids;

    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_path = params.metrics_path;
    #[cfg(feature = "gateway-metrics-prometheus")]
    let metrics_timer_start = params.metrics_timer_start;

            let batch_cancel_id = translation::batches_cancel_id(path_and_query);
            let batch_retrieve_id = translation::batches_retrieve_id(path_and_query);
            let batches_root = translation::is_batches_path(path_and_query);
            let models_root = translation::is_models_path(path_and_query);
            let models_retrieve_id = translation::models_retrieve_id(path_and_query);
            let files_root = translation::is_files_path(path_and_query);
            let files_retrieve_id = translation::files_retrieve_id(path_and_query);
            let files_content_id = translation::files_content_id(path_and_query);

            let supported_path = translation::is_chat_completions_path(path_and_query)
                || translation::is_completions_path(path_and_query)
                || models_root
                || translation::is_responses_create_path(path_and_query)
                || translation::is_responses_compact_path(path_and_query)
                || translation::is_embeddings_path(path_and_query)
                || translation::is_moderations_path(path_and_query)
                || translation::is_images_generations_path(path_and_query)
                || translation::is_audio_transcriptions_path(path_and_query)
                || translation::is_audio_translations_path(path_and_query)
                || translation::is_audio_speech_path(path_and_query)
                || translation::is_rerank_path(path_and_query)
                || batches_root
                || files_root
                || files_retrieve_id.is_some()
                || files_content_id.is_some()
                || batch_cancel_id.is_some()
                || batch_retrieve_id.is_some()
                || models_retrieve_id.is_some();

            let supported_method = if parts.method == axum::http::Method::POST {
                translation::is_chat_completions_path(path_and_query)
                    || translation::is_completions_path(path_and_query)
                    || translation::is_responses_create_path(path_and_query)
                    || translation::is_responses_compact_path(path_and_query)
                    || translation::is_embeddings_path(path_and_query)
                    || translation::is_moderations_path(path_and_query)
                    || translation::is_images_generations_path(path_and_query)
                    || translation::is_audio_transcriptions_path(path_and_query)
                    || translation::is_audio_translations_path(path_and_query)
                    || translation::is_audio_speech_path(path_and_query)
                    || translation::is_rerank_path(path_and_query)
                    || batches_root
                    || files_root
                    || batch_cancel_id.is_some()
            } else if parts.method == axum::http::Method::GET {
                batches_root
                    || batch_retrieve_id.is_some()
                    || models_root
                    || models_retrieve_id.is_some()
                    || files_root
                    || files_retrieve_id.is_some()
                    || files_content_id.is_some()
            } else if parts.method == axum::http::Method::DELETE {
                files_retrieve_id.is_some()
            } else {
                false
            };

            if !supported_path || !supported_method {
                return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
                    StatusCode::NOT_IMPLEMENTED,
                    "invalid_request_error",
                    Some("unsupported_endpoint"),
                    format!(
                        "translation backend does not support {} {}",
                        parts.method, path_and_query
                    ),
                ))));
            }

            let mut proxy_permits = match try_acquire_proxy_permits(state, backend_name)? {
                ProxyPermitOutcome::Acquired(permits) => permits,
                ProxyPermitOutcome::BackendRateLimited(err) => {
                    return Ok(BackendAttemptOutcome::Continue(Some(err)));
                }
            };

            {
                let mut gateway = state.gateway.lock().await;
                gateway.observability.record_backend_call();
            }

            #[cfg(feature = "gateway-metrics-prometheus")]
            let backend_timer_start = std::time::Instant::now();

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
                let mut metrics = metrics.lock().await;
                metrics.record_proxy_backend_attempt(backend_name);
                metrics.record_proxy_backend_in_flight_inc(backend_name);
            }

            let default_spend = ProxySpend {
                tokens: u64::from(charge_tokens),
                cost_usd_micros: charge_cost_usd_micros,
            };

            let result: Result<
                (axum::response::Response, ProxySpend),
                (StatusCode, Json<OpenAiErrorResponse>),
            > = 'translation_backend_attempt: {
                #[allow(clippy::collapsible_else_if)]
                if models_root && parts.method == axum::http::Method::GET {
                    let models = translation::collect_models_from_translation_backends(
                        state.translation_backends.as_ref(),
                    );
                    let value = translation::models_list_to_openai(&models, _now_epoch_seconds);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_static("multi"),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if let Some(model_id) = models_retrieve_id.as_deref()
                    && parts.method == axum::http::Method::GET
                {
                    let models = translation::collect_models_from_translation_backends(
                        state.translation_backends.as_ref(),
                    );
                    let Some(owned_by) = models.get(model_id) else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::NOT_FOUND,
                            "invalid_request_error",
                            Some("model_not_found"),
                            format!("model {model_id} not found"),
                        ));
                    };

                    let value =
                        translation::model_to_openai(model_id, owned_by, _now_epoch_seconds);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(owned_by).unwrap_or_else(|_| {
                            axum::http::HeaderValue::from_static("enabled")
                        }),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if batches_root && parts.method == axum::http::Method::GET {
                    let mut limit: Option<u32> = None;
                    let mut after: Option<String> = None;
                    let query = parts.uri.query().unwrap_or_default();
                    for pair in query.split('&') {
                        let Some((key, value)) = pair.split_once('=') else {
                            continue;
                        };
                        if key == "limit" {
                            limit = value.parse::<u32>().ok();
                        } else if key == "after" {
                            let value = value.trim();
                            if !value.is_empty() {
                                after = Some(value.to_string());
                            }
                        }
                    }

                    let listed = match translation_backend.list_batches(limit, after).await {
                        Ok(listed) => listed,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value = translation::batch_list_response_to_openai(&listed);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if batches_root && parts.method == axum::http::Method::POST {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json",
                        ));
                    };

                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "batches endpoint does not support stream=true",
                        ));
                    }

                    let request = match translation::batches_create_request_to_request(parsed_json)
                    {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                    let created = match translation_backend.create_batch(request).await {
                        Ok(created) => created,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value = translation::batch_to_openai(&created.batch);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if let Some(batch_id) = batch_retrieve_id.as_deref() {
                    let retrieved = match translation_backend.retrieve_batch(batch_id).await {
                        Ok(retrieved) => retrieved,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value = translation::batch_to_openai(&retrieved.batch);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if let Some(batch_id) = batch_cancel_id.as_deref() {
                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "batches endpoint does not support stream=true",
                        ));
                    }

                    let cancelled = match translation_backend.cancel_batch(batch_id).await {
                        Ok(cancelled) => cancelled,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value = translation::batch_to_openai(&cancelled.batch);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_rerank_path(path_and_query) {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json",
                        ));
                    };

                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "rerank endpoint does not support stream=true",
                        ));
                    }

                    let mut request = match translation::rerank_request_to_request(parsed_json) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                    let Some(original_model) = request.model.clone() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    };

                    let mapped_model = translation_backend.map_model(&original_model);
                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }
                    request.model = Some(mapped_model.clone());

                    let reranked = match translation_backend.rerank(&mapped_model, request).await {
                        Ok(reranked) => reranked,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value = translation::rerank_response_to_openai(&reranked);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_audio_transcriptions_path(path_and_query)
                    || translation::is_audio_translations_path(path_and_query)
                {
                    let endpoint = if translation::is_audio_translations_path(path_and_query) {
                        "audio/translations"
                    } else {
                        "audio/transcriptions"
                    };

                    let Some(content_type) = parts
                        .headers
                        .get("content-type")
                        .and_then(|value| value.to_str().ok())
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            format!("{endpoint} request missing content-type"),
                        ));
                    };

                    if !content_type
                        .to_ascii_lowercase()
                        .starts_with("multipart/form-data")
                    {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            format!("{endpoint} request must be multipart/form-data"),
                        ));
                    }

                    let request = match translation::audio_transcriptions_request_to_request(
                        content_type,
                        body,
                    ) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                    let Some(original_model) = request.model.clone() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    };

                    let mapped_model = translation_backend.map_model(&original_model);
                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }

                    let request_format = request.response_format;
                    let mut request = request;
                    request.model = Some(mapped_model.clone());

                    let transcribed = match translation_backend
                        .transcribe_audio(&mapped_model, request)
                        .await
                    {
                        Ok(transcribed) => transcribed,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let (content_type, is_json) =
                        translation::transcription_format_to_content_type(request_format);
                    let bytes = if is_json {
                        let value = serde_json::json!({ "text": transcribed.text });
                        serde_json::to_vec(&value)
                            .map(Bytes::from)
                            .unwrap_or_else(|_| Bytes::from(value.to_string()))
                    } else {
                        Bytes::from(transcribed.text)
                    };

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_str(content_type).unwrap_or_else(|_| {
                            axum::http::HeaderValue::from_static("application/octet-stream")
                        }),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_audio_speech_path(path_and_query) {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json",
                        ));
                    };

                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "audio/speech endpoint does not support stream=true",
                        ));
                    }

                    let request = match translation::audio_speech_request_to_request(parsed_json) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                    let Some(original_model) = request.model.clone() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    };

                    let mapped_model = translation_backend.map_model(&original_model);
                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }

                    let request_format = request.response_format;
                    let mut request = request;
                    request.model = Some(mapped_model.clone());

                    let spoken = match translation_backend
                        .speak_audio(&mapped_model, request)
                        .await
                    {
                        Ok(spoken) => spoken,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let content_type = spoken.media_type.clone().unwrap_or_else(|| {
                        translation::speech_response_format_to_content_type(request_format)
                            .to_string()
                    });

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
                            axum::http::HeaderValue::from_static("application/octet-stream")
                        }),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(
                        Bytes::from(spoken.audio),
                        proxy_permits.take(),
                    );
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if translation::is_embeddings_path(path_and_query) {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json",
                        ));
                    };

                    let original_model = model.clone().unwrap_or_default();
                    let mapped_model = translation_backend.map_model(&original_model);

                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }
                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "embeddings endpoint does not support stream=true",
                        ));
                    }

                    let texts = match translation::embeddings_request_to_texts(parsed_json) {
                        Ok(texts) => texts,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                    let embeddings = match translation_backend.embed(&mapped_model, texts).await {
                        Ok(embeddings) => embeddings,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let value =
                        translation::embeddings_to_openai_response(embeddings, &original_model);
                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else {
                    include!("rest.rs")
                }
            };
            include!("post.rs")
}
