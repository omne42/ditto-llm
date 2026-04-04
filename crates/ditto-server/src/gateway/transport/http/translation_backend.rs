#[cfg(feature = "gateway-translation")]
use super::*;

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
// inlined from translation_backend/attempt.rs
#[cfg(feature = "gateway-translation")]
pub(super) async fn attempt_translation_backend(
    params: ProxyAttemptParams<'_>,
    backend_name: &str,
    translation_backend: super::super::TranslationBackend,
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

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    let token_budget_reservation_ids = params.token_budget_reservation_ids;

    let _cost_budget_reserved = params.cost_budget_reserved;
    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
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
    let responses_retrieve_id = translation::responses_retrieve_id(path_and_query);
    let responses_input_items_id = translation::responses_input_items_id(path_and_query);
    let responses_input_tokens = translation::is_responses_input_tokens_path(path_and_query);
    let videos_root = translation::is_videos_path(path_and_query);
    let videos_retrieve_id = translation::videos_retrieve_id(path_and_query);
    let videos_content_id = translation::videos_content_id(path_and_query);
    let videos_remix_id = translation::videos_remix_id(path_and_query);

    let Some(endpoint_descriptor) =
        translation::translation_endpoint_descriptor(&parts.method, path_and_query)
    else {
        return Ok(BackendAttemptOutcome::Continue(Some(openai_error(
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            Some("unsupported_endpoint"),
            format!(
                "translation backend does not support {} {}",
                parts.method, path_and_query
            ),
        ))));
    };

    let mapped_request_model = model
        .as_deref()
        .map(|requested_model| translation_backend.map_model(backend_name, requested_model))
        .filter(|requested_model| !requested_model.trim().is_empty());

    fn response_store_contract_message(response_id: &str) -> String {
        format!(
            "response {response_id} not found; translated response retrieval requires a gateway-scoped id from a non-streaming /v1/responses create on the same gateway instance"
        )
    }
    if !translation_backend.supports_endpoint(&endpoint_descriptor, mapped_request_model.as_deref())
    {
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

    state.record_backend_call();

    #[cfg(feature = "gateway-metrics-prometheus")]
    let backend_timer_start = std::time::Instant::now();

    #[cfg(feature = "gateway-metrics-prometheus")]
    if let Some(metrics) = state.proxy.metrics.as_ref() {
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
            let models =
                translation::collect_models_from_translation_backend(backend_name, &translation_backend);
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
                axum::http::HeaderValue::from_str(backend_name)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
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
            let models =
                translation::collect_models_from_translation_backend(backend_name, &translation_backend);
            let Some(owned_by) = models.get(model_id) else {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::NOT_FOUND,
                    "invalid_request_error",
                    Some("model_not_found"),
                    format!("model {model_id} not found"),
                ));
            };

            let value = translation::model_to_openai(model_id, owned_by, _now_epoch_seconds);
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
                axum::http::HeaderValue::from_str(backend_name)
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
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

            let request = match translation::batches_create_request_to_request(parsed_json) {
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

            let mapped_model = translation_backend.map_model(backend_name, &original_model);
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

            let request =
                match translation::audio_transcriptions_request_to_request(content_type, body) {
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

            let mapped_model = translation_backend.map_model(backend_name, &original_model);
            if mapped_model.trim().is_empty() {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    Some("invalid_request"),
                    "missing model",
                ));
            }
            if !translation_backend
                .supports_endpoint(&endpoint_descriptor, Some(mapped_model.as_str()))
            {
                break 'translation_backend_attempt Err(openai_error(
                    StatusCode::NOT_IMPLEMENTED,
                    "invalid_request_error",
                    Some("unsupported_endpoint"),
                    format!(
                        "translation backend does not support {} {}",
                        parts.method, path_and_query
                    ),
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

            let mapped_model = translation_backend.map_model(backend_name, &original_model);
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
                translation::speech_response_format_to_content_type(request_format).to_string()
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

            let body =
                proxy_body_from_bytes_with_permit(Bytes::from(spoken.audio), proxy_permits.take());
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
            let mapped_model = translation_backend.map_model(backend_name, &original_model);

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

            let value = translation::embeddings_to_openai_response(embeddings, &original_model);
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
            // inlined from rest.rs
            if translation::is_moderations_path(path_and_query) {
                let Some(request_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "moderations endpoint does not support stream=true",
                    ));
                }

                let mut request = match translation::moderations_request_to_request(request_json) {
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

                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let moderated = match translation_backend.moderate(request).await {
                    Ok(moderated) => moderated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let fallback_id = format!("modr_{request_id}");
                let value = translation::moderation_response_to_openai(&moderated, &fallback_id);

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
            } else if files_root && parts.method == axum::http::Method::POST {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files request missing content-type",
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
                        "files request must be multipart/form-data",
                    ));
                }

                let request = match translation::files_upload_request_to_request(content_type, body)
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

                let bytes_len = request.bytes.len();
                let filename = request.filename.clone();
                let purpose = request.purpose.clone();
                let file_id = match translation_backend.upload_file(request).await {
                    Ok(file_id) => file_id,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::file_upload_response_to_openai(
                    &file_id,
                    filename,
                    purpose,
                    bytes_len,
                    _now_epoch_seconds,
                );
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
            } else if files_root && parts.method == axum::http::Method::GET {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let files = match translation_backend.list_files().await {
                    Ok(files) => files,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::file_list_response_to_openai(&files);
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
            } else if let Some(file_id) = files_content_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let content = match translation_backend.download_file_content(file_id).await {
                    Ok(content) => content,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let content_type = content
                    .media_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());

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
                    Bytes::from(content.bytes),
                    proxy_permits.take(),
                );
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(file_id) = files_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "files endpoint does not support stream=true",
                    ));
                }

                let value = if parts.method == axum::http::Method::GET {
                    let file = match translation_backend.retrieve_file(file_id).await {
                        Ok(file) => file,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::file_to_openai(&file)
                } else if parts.method == axum::http::Method::DELETE {
                    let deleted = match translation_backend.delete_file(file_id).await {
                        Ok(deleted) => deleted,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::file_delete_response_to_openai(&deleted)
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                };

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
            } else if videos_root && parts.method == axum::http::Method::POST {
                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos request missing content-type",
                    ));
                };

                let multipart_stream_requested = if content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    match translation::multipart_extract_text_field(content_type, body, "stream") {
                        Ok(Some(value)) => matches!(value.trim(), "true" | "1"),
                        Ok(None) => false,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    }
                } else {
                    false
                };

                if _stream_requested || multipart_stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let mut request = if content_type
                    .to_ascii_lowercase()
                    .starts_with("multipart/form-data")
                {
                    match translation::videos_create_multipart_request_to_request(
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
                    }
                } else {
                    let Some(parsed_json) = parsed_json.as_ref() else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "request body must be application/json or multipart/form-data",
                        ));
                    };
                    match translation::videos_create_request_to_request(parsed_json) {
                        Ok(request) => request,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    }
                };

                let original_model = request.model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);
                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let generated = match translation_backend.create_video(request).await {
                    Ok(generated) => generated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_generation_response_to_openai(&generated);
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
            } else if videos_root && parts.method == axum::http::Method::GET {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let request = match translation::videos_list_request_from_path(path_and_query) {
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

                let videos = match translation_backend.list_videos(request).await {
                    Ok(videos) => videos,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_list_response_to_openai(&videos);
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
            } else if let Some(video_id) = videos_content_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let variant = match translation::videos_content_variant_from_path(path_and_query) {
                    Ok(variant) => variant,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let content = match translation_backend
                    .download_video_content(video_id, variant)
                    .await
                {
                    Ok(content) => content,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let content_type = content
                    .media_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());

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
                    Bytes::from(content.bytes),
                    proxy_permits.take(),
                );
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(video_id) = videos_remix_id.as_deref()
                && parts.method == axum::http::Method::POST
            {
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
                        "videos endpoint does not support stream=true",
                    ));
                }

                let request = match translation::videos_remix_request_to_request(parsed_json) {
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

                let remixed = match translation_backend.remix_video(video_id, request).await {
                    Ok(remixed) => remixed,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::video_generation_response_to_openai(&remixed);
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
            } else if let Some(video_id) = videos_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "videos endpoint does not support stream=true",
                    ));
                }

                let value = if parts.method == axum::http::Method::GET {
                    let video = match translation_backend.retrieve_video(video_id).await {
                        Ok(video) => video,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::video_generation_response_to_openai(&video)
                } else if parts.method == axum::http::Method::DELETE {
                    let deleted = match translation_backend.delete_video(video_id).await {
                        Ok(deleted) => deleted,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    translation::video_delete_response_to_openai(&deleted)
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                };

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
            } else if responses_input_tokens && parts.method == axum::http::Method::POST {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses input_tokens endpoint does not support stream=true",
                    ));
                }

                let Some(request_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                if original_model.trim().is_empty() {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses input_tokens endpoint requires model",
                    ));
                }

                let mapped_model = translation_backend.map_model(backend_name, &original_model);

                #[cfg(feature = "gateway-tokenizer")]
                let input_tokens = {
                    let tokenizer_model = mapped_model
                        .trim()
                        .split_once('/')
                        .map(|(_, model)| model)
                        .unwrap_or_else(|| mapped_model.trim());
                    let tokenizer_model = if tokenizer_model.is_empty() {
                        original_model.trim()
                    } else {
                        tokenizer_model
                    };
                    token_count::estimate_input_tokens(
                        "/v1/responses",
                        tokenizer_model,
                        request_json,
                    )
                    .unwrap_or_else(|| estimate_tokens_from_bytes(body))
                };

                #[cfg(not(feature = "gateway-tokenizer"))]
                {
                    let _ = (&mapped_model, &original_model, request_json);
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        "responses input_tokens endpoint requires gateway-tokenizer feature",
                    ));
                }

                #[cfg(feature = "gateway-tokenizer")]
                {
                    let value = translation::responses_input_tokens_to_openai(input_tokens);
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
                    Ok((
                        response,
                        ProxySpend {
                            tokens: 0,
                            cost_usd_micros: None,
                        },
                    ))
                }
            } else if let Some(response_id) = responses_input_items_id.as_deref()
                && parts.method == axum::http::Method::GET
            {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses endpoint does not support stream=true",
                    ));
                }

                let Some((stored_backend_name, stored_response)) =
                    translation::find_stored_response_from_translation_backends(
                        state.backends.translation_backends.as_ref(),
                        response_id,
                    )
                    .await
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_FOUND,
                        "invalid_request_error",
                        Some("response_not_found"),
                        response_store_contract_message(response_id),
                    ));
                };

                let value =
                    translation::responses_input_items_to_openai(&stored_response.input_items);
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
                    axum::http::HeaderValue::from_str(&stored_backend_name)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                );
                apply_proxy_response_headers(&mut headers, &stored_backend_name, request_id, false);

                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = StatusCode::OK;
                *response.headers_mut() = headers;
                Ok((response, default_spend))
            } else if let Some(response_id) = responses_retrieve_id.as_deref() {
                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "responses endpoint does not support stream=true",
                    ));
                }

                if parts.method == axum::http::Method::GET {
                    let Some((stored_backend_name, stored_response)) =
                        translation::find_stored_response_from_translation_backends(
                            state.backends.translation_backends.as_ref(),
                            response_id,
                        )
                        .await
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::NOT_FOUND,
                            "invalid_request_error",
                            Some("response_not_found"),
                            response_store_contract_message(response_id),
                        ));
                    };

                    let bytes = serde_json::to_vec(&stored_response.response)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(stored_response.response.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&stored_backend_name)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(
                        &mut headers,
                        &stored_backend_name,
                        request_id,
                        false,
                    );

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else if parts.method == axum::http::Method::DELETE {
                    let Some(stored_backend_name) =
                        translation::delete_stored_response_from_translation_backends(
                            state.backends.translation_backends.as_ref(),
                            response_id,
                        )
                        .await
                    else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::NOT_FOUND,
                            "invalid_request_error",
                            Some("response_not_found"),
                            response_store_contract_message(response_id),
                        ));
                    };

                    let value = translation::response_delete_to_openai(response_id);
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
                        axum::http::HeaderValue::from_str(&stored_backend_name)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    apply_proxy_response_headers(
                        &mut headers,
                        &stored_backend_name,
                        request_id,
                        false,
                    );

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::NOT_IMPLEMENTED,
                        "invalid_request_error",
                        Some("unsupported_endpoint"),
                        format!(
                            "translation backend does not support {} {}",
                            parts.method, path_and_query
                        ),
                    ));
                }
            } else if translation::is_responses_compact_path(path_and_query) {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);

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
                        "responses/compact endpoint does not support stream=true",
                    ));
                }

                let instructions = parsed_json
                    .get("instructions")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();

                let Some(input) = parsed_json.get("input") else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "missing input",
                    ));
                };

                let input_items = match translation::responses_input_items_from_value(input) {
                    Ok(items) => items,
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let (output, usage) = match translation_backend
                    .compact_responses_history(&mapped_model, instructions, &input_items)
                    .await
                {
                    Ok(compacted) => compacted,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = serde_json::json!({ "output": output });
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

                let tokens = usage
                    .total_tokens
                    .unwrap_or_else(|| u64::from(charge_tokens));
                #[cfg(feature = "gateway-costing")]
                let cost_usd_micros = model.as_deref().and_then(|model| {
                    state.proxy.pricing.as_ref().and_then(|pricing| {
                        let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens)
                        else {
                            return None;
                        };
                        pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                            model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                            service_tier.as_deref(),
                        )
                    })
                });
                #[cfg(not(feature = "gateway-costing"))]
                let cost_usd_micros: Option<u64> = None;

                Ok((
                    response,
                    ProxySpend {
                        tokens,
                        cost_usd_micros: cost_usd_micros.or(charge_cost_usd_micros),
                    },
                ))
            } else if translation::is_images_edits_path(path_and_query) {
                let Some(content_type) = parts
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images/edits request missing content-type",
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
                        "images/edits request must be multipart/form-data",
                    ));
                }

                let multipart_stream_requested =
                    match translation::multipart_extract_text_field(content_type, body, "stream") {
                        Ok(Some(value)) => matches!(value.trim(), "true" | "1"),
                        Ok(None) => false,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    };

                if _stream_requested || multipart_stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images endpoint does not support stream=true",
                    ));
                }

                let mut request =
                    match translation::images_edits_request_to_request(content_type, body) {
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

                let original_model = request.model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);
                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let edited = match translation_backend.edit_image(request).await {
                    Ok(edited) => edited,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value =
                    translation::image_generation_response_to_openai(&edited, _now_epoch_seconds);
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
            } else if translation::is_images_generations_path(path_and_query) {
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);

                if _stream_requested {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "images endpoint does not support stream=true",
                    ));
                }

                let mut request =
                    match translation::images_generation_request_to_request(parsed_json) {
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

                if !mapped_model.trim().is_empty() {
                    request.model = Some(mapped_model);
                }

                let generated = match translation_backend.generate_image(request).await {
                    Ok(generated) => generated,
                    Err(err) => {
                        let (status, kind, code, message) =
                            translation::map_provider_error_to_openai(err);
                        break 'translation_backend_attempt Err(openai_error(
                            status, kind, code, message,
                        ));
                    }
                };

                let value = translation::image_generation_response_to_openai(
                    &generated,
                    _now_epoch_seconds,
                );
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
                let Some(parsed_json) = parsed_json.as_ref() else {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "request body must be application/json",
                    ));
                };

                let original_model = model.clone().unwrap_or_default();
                let mapped_model = translation_backend.map_model(backend_name, &original_model);

                if mapped_model.trim().is_empty() {
                    break 'translation_backend_attempt Err(openai_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        Some("invalid_request"),
                        "missing model",
                    ));
                }

                let responses_input_items = if translation::is_responses_create_path(path_and_query)
                {
                    let Some(input) = parsed_json.get("input") else {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing input",
                        ));
                    };
                    Some(match translation::responses_input_items_from_value(input) {
                        Ok(items) => items,
                        Err(err) => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                err,
                            ));
                        }
                    })
                } else {
                    None
                };

                let generate_request = if translation::is_chat_completions_path(path_and_query) {
                    translation::chat_completions_request_to_generate_request(parsed_json)
                } else if translation::is_completions_path(path_and_query) {
                    translation::completions_request_to_generate_request(parsed_json)
                } else {
                    translation::responses_request_to_generate_request(parsed_json)
                };

                let generate_request = match generate_request {
                    Ok(mut request) => {
                        request.model = Some(mapped_model);
                        request
                    }
                    Err(err) => {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            err,
                        ));
                    }
                };

                let fallback_response_id = if translation::is_chat_completions_path(path_and_query)
                {
                    format!("chatcmpl_{request_id}")
                } else if translation::is_completions_path(path_and_query) {
                    format!("cmpl_{request_id}")
                } else {
                    format!("resp_{request_id}")
                };

                let include_usage = _stream_requested
                    && translation::is_chat_completions_path(path_and_query)
                    && parsed_json
                        .get("stream_options")
                        .and_then(|value| value.get("include_usage"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);

                if _stream_requested {
                    let stream = match translation_backend.model.stream(generate_request).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let stream = if translation::is_chat_completions_path(path_and_query) {
                        translation::stream_to_chat_completions_sse(
                            stream,
                            fallback_response_id.clone(),
                            original_model.clone(),
                            _now_epoch_seconds,
                            include_usage,
                        )
                    } else if translation::is_completions_path(path_and_query) {
                        translation::stream_to_completions_sse(
                            stream,
                            fallback_response_id.clone(),
                            original_model.clone(),
                            _now_epoch_seconds,
                        )
                    } else {
                        translation::stream_to_responses_sse(stream, fallback_response_id)
                    };

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("text/event-stream"),
                    );
                    headers.insert(
                        "x-ditto-translation",
                        axum::http::HeaderValue::from_str(&translation_backend.provider)
                            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("enabled")),
                    );
                    headers.remove("content-length");
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let stream = ProxyBodyStreamWithPermit {
                        inner: stream.boxed(),
                        _permits: proxy_permits.take(),
                    };
                    let mut response = axum::response::Response::new(Body::from_stream(stream));
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
                } else {
                    let generated = match translation_backend.model.generate(generate_request).await
                    {
                        Ok(generated) => generated,
                        Err(err) => {
                            let (status, kind, code, message) =
                                translation::map_provider_error_to_openai(err);
                            break 'translation_backend_attempt Err(openai_error(
                                status, kind, code, message,
                            ));
                        }
                    };

                    let provider_response_id =
                        translation::provider_response_id(&generated, &fallback_response_id);
                    let response_id = if translation::is_responses_path(path_and_query) {
                        translation::gateway_scoped_response_id(
                            backend_name,
                            &provider_response_id,
                        )
                    } else {
                        provider_response_id
                    };
                    let value = if translation::is_chat_completions_path(path_and_query) {
                        translation::generate_response_to_chat_completions(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    } else if translation::is_completions_path(path_and_query) {
                        translation::generate_response_to_completions(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    } else {
                        translation::generate_response_to_responses(
                            &generated,
                            &response_id,
                            &original_model,
                            _now_epoch_seconds,
                        )
                    };

                    if let Some(input_items) = responses_input_items {
                        translation_backend
                            .store_response_record(&response_id, value.clone(), input_items)
                            .await;
                    }

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
                    let mut usage = generated.usage;
                    usage.merge_total();
                    let tokens = usage
                        .total_tokens
                        .unwrap_or_else(|| u64::from(charge_tokens));
                    #[cfg(feature = "gateway-costing")]
                    let cost_usd_micros = model.as_deref().and_then(|model| {
                        state.proxy.pricing.as_ref().and_then(|pricing| {
                            let (Some(input), Some(output)) =
                                (usage.input_tokens, usage.output_tokens)
                            else {
                                return None;
                            };
                            pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                                model,
                                clamp_u64_to_u32(input),
                                usage.cache_input_tokens.map(clamp_u64_to_u32),
                                usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                                clamp_u64_to_u32(output),
                                service_tier.as_deref(),
                            )
                        })
                    });
                    #[cfg(not(feature = "gateway-costing"))]
                    let cost_usd_micros: Option<u64> = None;
                    Ok((
                        response,
                        ProxySpend {
                            tokens,
                            cost_usd_micros: cost_usd_micros.or(charge_cost_usd_micros),
                        },
                    ))
                }
            }
            // end inline: rest.rs
        }
    };
    // inlined from post.rs
    {
        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let mut metrics = metrics.lock().await;
            metrics.record_proxy_backend_in_flight_dec(backend_name);
            metrics.observe_proxy_backend_request_duration(
                backend_name,
                backend_timer_start.elapsed(),
            );
        }

        let (response, spend) = match result {
            Ok((response, spend)) => (response, spend),
            Err(err) => {
                return Ok(BackendAttemptOutcome::Continue(Some(err)));
            }
        };

        let status = StatusCode::OK;
        let spend_tokens = true;
        let spent_tokens = spend.tokens;
        let spent_cost_usd_micros = spend.cost_usd_micros;
        #[cfg(not(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-redis",
            feature = "gateway-costing",
        )))]
        let _ = spent_cost_usd_micros;

        #[cfg(feature = "gateway-metrics-prometheus")]
        if let Some(metrics) = state.proxy.metrics.as_ref() {
            let duration = metrics_timer_start.elapsed();
            let mut metrics = metrics.lock().await;
            if spend_tokens {
                metrics.record_proxy_backend_success(backend_name);
            } else {
                metrics.record_proxy_backend_failure(backend_name);
            }
            metrics.record_proxy_response_status_by_path(metrics_path, status.as_u16());
            metrics.record_proxy_response_status_by_backend(backend_name, status.as_u16());
            if let Some(model) = model.as_deref() {
                metrics.record_proxy_response_status_by_model(model, status.as_u16());
                metrics.observe_proxy_request_duration_by_model(model, duration);
            }
            metrics.observe_proxy_request_duration(metrics_path, duration);
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        if !token_budget_reservation_ids.is_empty() {
            settle_proxy_token_budget_reservations(
                state,
                token_budget_reservation_ids,
                spend_tokens,
                spent_tokens,
            )
            .await;
        } else if let (Some(virtual_key_id), Some(budget)) =
            (virtual_key_id.clone(), budget.clone())
            && spend_tokens
        {
            state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
            if let Some((scope, budget)) = project_budget_scope.as_ref() {
                state.spend_budget_tokens(scope, budget, spent_tokens);
            }
            if let Some((scope, budget)) = user_budget_scope.as_ref() {
                state.spend_budget_tokens(scope, budget, spent_tokens);
            }

            #[cfg(feature = "gateway-costing")]
            if !use_persistent_budget && let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                }
            }
        }
        #[cfg(not(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        )))]
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            if spend_tokens {
                state.spend_budget_tokens(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    state.spend_budget_tokens(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    state.spend_budget_cost(&virtual_key_id, &budget, spent_cost_usd_micros);
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        state.spend_budget_cost(scope, budget, spent_cost_usd_micros);
                    }
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !cost_budget_reservation_ids.is_empty() {
            settle_proxy_cost_budget_reservations(
                state,
                cost_budget_reservation_ids,
                spend_tokens,
                spent_cost_usd_micros.unwrap_or_default(),
            )
            .await;
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(
                feature = "gateway-store-sqlite",
                feature = "gateway-store-postgres",
                feature = "gateway-store-mysql",
                feature = "gateway-store-redis"
            ),
        ))]
        if !_cost_budget_reserved
            && use_persistent_budget
            && spend_tokens
            && let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
        {
            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.stores.sqlite.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
            #[cfg(feature = "gateway-store-postgres")]
            if let Some(store) = state.stores.postgres.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
            #[cfg(feature = "gateway-store-mysql")]
            if let Some(store) = state.stores.mysql.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.stores.redis.as_ref() {
                let _ = store
                    .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                    .await;
            }
        }

        #[cfg(any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ))]
        {
            let payload = serde_json::json!({
                "request_id": &request_id,
                "virtual_key_id": virtual_key_id.as_deref(),
                "backend": &backend_name,
                "attempted_backends": &attempted_backends,
                "method": parts.method.as_str(),
                "path": path_and_query,
                "model": &model,
                "status": status.as_u16(),
                "charge_tokens": charge_tokens,
                "spent_tokens": spent_tokens,
                "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
                "mode": "translation",
            });
            append_audit_log(state, "proxy", payload).await;
        }

        emit_json_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
                "mode": "translation",
            }),
        );

        #[cfg(feature = "sdk")]
        emit_devtools_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "status": status.as_u16(),
                "path": path_and_query,
                "backend": &backend_name,
                "mode": "translation",
            }),
        );

        Ok(BackendAttemptOutcome::Response(response))
    }
    // end inline: post.rs
}
// end inline: translation_backend/attempt.rs
