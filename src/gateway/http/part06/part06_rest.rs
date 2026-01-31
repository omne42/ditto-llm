                if translation::is_moderations_path(path_and_query) {
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

                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "moderations endpoint does not support stream=true",
                        ));
                    }

                    let mut request = match translation::moderations_request_to_request(parsed_json)
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
                    let value =
                        translation::moderation_response_to_openai(&moderated, &fallback_id);

                    let bytes = serde_json::to_vec(&value)
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(value.to_string()));

                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;
                    Ok((response, default_spend))
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

                    let input_items = match input {
                        Value::Array(items) => items.clone(),
                        Value::Object(_) => vec![input.clone()],
                        Value::String(text) => vec![serde_json::json!({
                            "type":"message",
                            "role":"user",
                            "content":[{"type":"input_text","text": text}],
                        })],
                        _ => {
                            break 'translation_backend_attempt Err(openai_error(
                                StatusCode::BAD_REQUEST,
                                "invalid_request_error",
                                Some("invalid_request"),
                                "`input` must be a string, array, or object",
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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
                    );
                    apply_proxy_response_headers(&mut headers, backend_name, request_id, false);

                    let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                    let mut response = axum::response::Response::new(body);
                    *response.status_mut() = StatusCode::OK;
                    *response.headers_mut() = headers;

                    let tokens = usage.total_tokens.unwrap_or(u64::from(charge_tokens));
                    #[cfg(feature = "gateway-costing")]
                    let cost_usd_micros = model.as_deref().and_then(|model| {
                        state.pricing.as_ref().and_then(|pricing| {
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
                    let mapped_model = translation_backend.map_model(&original_model);

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
                    headers.insert("content-type", "application/json".parse().unwrap());
                    headers.insert(
                        "x-ditto-translation",
                        translation_backend
                            .provider
                            .parse()
                            .unwrap_or_else(|_| "enabled".parse().unwrap()),
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
                    let mapped_model = translation_backend.map_model(&original_model);

                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }

                    let generate_request = if translation::is_chat_completions_path(path_and_query)
                    {
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

                    let fallback_response_id =
                        if translation::is_chat_completions_path(path_and_query) {
                            format!("chatcmpl_{request_id}")
                        } else if translation::is_completions_path(path_and_query) {
                            format!("cmpl_{request_id}")
                        } else {
                            format!("resp_{request_id}")
                        };

                    if _stream_requested {
                        let stream = match translation_backend.model.stream(generate_request).await
                        {
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
                        headers.insert("content-type", "text/event-stream".parse().unwrap());
                        headers.insert(
                            "x-ditto-translation",
                            translation_backend
                                .provider
                                .parse()
                                .unwrap_or_else(|_| "enabled".parse().unwrap()),
                        );
                        headers.remove("content-length");
                        apply_proxy_response_headers(
                            &mut headers,
                            backend_name,
                            request_id,
                            false,
                        );

                        let stream = ProxyBodyStreamWithPermit {
                            inner: stream.boxed(),
                            _permits: proxy_permits.take(),
                        };
                        let mut response = axum::response::Response::new(Body::from_stream(stream));
                        *response.status_mut() = StatusCode::OK;
                        *response.headers_mut() = headers;
                        Ok((response, default_spend))
                    } else {
                        let generated =
                            match translation_backend.model.generate(generate_request).await {
                                Ok(generated) => generated,
                                Err(err) => {
                                    let (status, kind, code, message) =
                                        translation::map_provider_error_to_openai(err);
                                    break 'translation_backend_attempt Err(openai_error(
                                        status, kind, code, message,
                                    ));
                                }
                            };

                        let response_id =
                            translation::provider_response_id(&generated, &fallback_response_id);
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

                        let bytes = serde_json::to_vec(&value)
                            .map(Bytes::from)
                            .unwrap_or_else(|_| Bytes::from(value.to_string()));

                        let mut headers = HeaderMap::new();
                        headers.insert("content-type", "application/json".parse().unwrap());
                        headers.insert(
                            "x-ditto-translation",
                            translation_backend
                                .provider
                                .parse()
                                .unwrap_or_else(|_| "enabled".parse().unwrap()),
                        );
                        apply_proxy_response_headers(
                            &mut headers,
                            backend_name,
                            request_id,
                            false,
                        );

                        let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                        let mut response = axum::response::Response::new(body);
                        *response.status_mut() = StatusCode::OK;
                        *response.headers_mut() = headers;
                        let mut usage = generated.usage.clone();
                        usage.merge_total();
                        let tokens = usage.total_tokens.unwrap_or(u64::from(charge_tokens));
                        #[cfg(feature = "gateway-costing")]
                        let cost_usd_micros = model.as_deref().and_then(|model| {
                            state.pricing.as_ref().and_then(|pricing| {
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
