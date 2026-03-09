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
                    let mapped_model = translation_backend.map_model(&original_model);

                    if _stream_requested {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "moderations endpoint does not support stream=true",
                        ));
                    }

                    let mut request = match translation::moderations_request_to_request(request_json)
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

                    let request =
                        match translation::files_upload_request_to_request(content_type, body) {
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
                            format!("translation backend does not support {} {}", parts.method, path_and_query),
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
                        match translation::videos_create_multipart_request_to_request(content_type, body) {
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
                    let mapped_model = translation_backend.map_model(&original_model);
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
                            format!("translation backend does not support {} {}", parts.method, path_and_query),
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

                    let mapped_model = translation_backend.map_model(&original_model);

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

                    let Some((stored_backend_name, stored_provider, stored_response)) =
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
                            format!("response {response_id} not found"),
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
                        axum::http::HeaderValue::from_str(&stored_provider)
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
                        let Some((stored_backend_name, stored_provider, stored_response)) =
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
                                format!("response {response_id} not found"),
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
                            axum::http::HeaderValue::from_str(&stored_provider).unwrap_or_else(|_| {
                                axum::http::HeaderValue::from_static("enabled")
                            }),
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
                        let Some((stored_backend_name, stored_provider)) =
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
                                format!("response {response_id} not found"),
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
                            axum::http::HeaderValue::from_str(&stored_provider).unwrap_or_else(|_| {
                                axum::http::HeaderValue::from_static("enabled")
                            }),
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
                        match translation::multipart_extract_text_field(content_type, body, "stream")
                        {
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
                    let mapped_model = translation_backend.map_model(&original_model);
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
                    let mapped_model = translation_backend.map_model(&original_model);

                    if mapped_model.trim().is_empty() {
                        break 'translation_backend_attempt Err(openai_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            Some("invalid_request"),
                            "missing model",
                        ));
                    }

                    let responses_input_items = if translation::is_responses_create_path(path_and_query) {
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

                    let include_usage = _stream_requested
                        && translation::is_chat_completions_path(path_and_query)
                        && parsed_json
                            .get("stream_options")
                            .and_then(|value| value.get("include_usage"))
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false);

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
                                .unwrap_or_else(|_| {
                                    axum::http::HeaderValue::from_static("enabled")
                                }),
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
                                .unwrap_or_else(|_| {
                                    axum::http::HeaderValue::from_static("enabled")
                                }),
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
