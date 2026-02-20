{
        let content_length = upstream_headers
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());

        #[cfg(feature = "gateway-proxy-cache")]
        let should_attempt_buffer_for_cache =
            status.is_success() && proxy_cache_key.is_some() && state.proxy_cache_config.is_some();

        let should_attempt_buffer_for_usage =
            content_type.starts_with("application/json") && state.proxy_usage_max_body_bytes > 0;

        let cache_max_buffer_bytes = {
            #[cfg(feature = "gateway-proxy-cache")]
            {
                if should_attempt_buffer_for_cache {
                    state
                        .proxy_cache_config
                        .as_ref()
                        .map(|config| config.max_body_bytes)
                        .unwrap_or(1024 * 1024)
                } else {
                    0
                }
            }
            #[cfg(not(feature = "gateway-proxy-cache"))]
            {
                0
            }
        };

        let usage_max_buffer_bytes = if should_attempt_buffer_for_usage {
            state.proxy_usage_max_body_bytes
        } else {
            0
        };

        let max_buffer_bytes = cache_max_buffer_bytes.max(usage_max_buffer_bytes);
        let should_try_buffer =
            max_buffer_bytes > 0 && content_length.is_none_or(|len| len <= max_buffer_bytes);

        enum ProxyResponseBody {
            Bytes(Bytes),
            Stream(ProxyBodyStream),
        }

        let response_body = if should_try_buffer {
            let mut upstream_stream = upstream_response.bytes_stream();
            let initial_capacity = content_length
                .map(|len| len.min(max_buffer_bytes))
                .unwrap_or(0);
            let mut buffered = bytes::BytesMut::with_capacity(initial_capacity);
            let mut first_unbuffered: Option<Bytes> = None;
            let mut stream_error: Option<std::io::Error> = None;

            while let Some(next) = upstream_stream.next().await {
                match next {
                    Ok(chunk) => {
                        if buffered.len().saturating_add(chunk.len()) <= max_buffer_bytes {
                            buffered.extend_from_slice(chunk.as_ref());
                        } else {
                            first_unbuffered = Some(chunk);
                            break;
                        }
                    }
                    Err(err) => {
                        stream_error = Some(std::io::Error::other(err));
                        break;
                    }
                }
            }

            match (first_unbuffered, stream_error) {
                (None, None) => ProxyResponseBody::Bytes(buffered.freeze()),
                (Some(chunk), _) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let first = futures_util::stream::once(async move {
                        Ok::<Bytes, std::io::Error>(chunk)
                    });
                    let rest = upstream_stream.map(|chunk| chunk.map_err(std::io::Error::other));
                    let stream = prefix.chain(first).chain(rest).boxed();
                    ProxyResponseBody::Stream(stream)
                }
                (None, Some(err)) => {
                    let prefix_bytes = buffered.freeze();
                    let prefix: ProxyBodyStream = if prefix_bytes.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move {
                            Ok::<Bytes, std::io::Error>(prefix_bytes)
                        })
                        .boxed()
                    };
                    let err_stream =
                        futures_util::stream::once(async move { Err::<Bytes, std::io::Error>(err) });
                    let stream = prefix.chain(err_stream).boxed();
                    ProxyResponseBody::Stream(stream)
                }
            }
        } else {
            let stream = upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other))
                .boxed();
            ProxyResponseBody::Stream(stream)
        };

        let observed_usage = if should_attempt_buffer_for_usage {
            match &response_body {
                ProxyResponseBody::Bytes(bytes) => extract_openai_usage_from_bytes(bytes),
                ProxyResponseBody::Stream(_) => None,
            }
        } else {
            None
        };

        let spent_tokens = if spend_tokens {
            observed_usage
                .and_then(|usage| usage.total_tokens)
                .unwrap_or_else(|| u64::from(charge_tokens))
        } else {
            0
        };

        #[cfg(feature = "gateway-costing")]
        let spent_cost_usd_micros = if spend_tokens {
            model
                .as_deref()
                .map(|request_model| {
                    backend_model_map
                        .get(request_model)
                        .map(|model| model.as_str())
                        .unwrap_or(request_model)
                })
                .and_then(|cost_model| {
                    state.pricing.as_ref().and_then(|pricing| {
                        let usage = observed_usage?;
                        let input = usage.input_tokens?;
                        let output = usage.output_tokens?;
                        pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                            cost_model,
                            clamp_u64_to_u32(input),
                            usage.cache_input_tokens.map(clamp_u64_to_u32),
                            usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                            clamp_u64_to_u32(output),
                            service_tier.as_deref(),
                        )
                    })
                })
                .or(charge_cost_usd_micros)
        } else {
            None
        };
        #[cfg(not(feature = "gateway-costing"))]
        let spent_cost_usd_micros: Option<u64> = None;

        #[cfg(not(any(
            feature = "gateway-costing",
            feature = "gateway-store-sqlite",
            feature = "gateway-store-redis"
        )))]
        let _ = spent_cost_usd_micros;

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
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
        {
            if spend_tokens {
                let mut gateway = state.gateway.lock().await;
                gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if !use_persistent_budget {
                    if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                        gateway.budget.spend_cost_usd_micros(
                            &virtual_key_id,
                            &budget,
                            spent_cost_usd_micros,
                        );
                        if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                        if let Some((scope, budget)) = project_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                        if let Some((scope, budget)) = user_budget_scope.as_ref() {
                            gateway.budget.spend_cost_usd_micros(
                                scope,
                                budget,
                                spent_cost_usd_micros,
                            );
                        }
                    }
                }
            }
        }
        #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
        if let (Some(virtual_key_id), Some(budget)) = (virtual_key_id.clone(), budget.clone()) {
            if spend_tokens {
                let mut gateway = state.gateway.lock().await;
                gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = project_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }
                if let Some((scope, budget)) = user_budget_scope.as_ref() {
                    gateway.budget.spend(scope, budget, spent_tokens);
                }

                #[cfg(feature = "gateway-costing")]
                if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                    gateway.budget.spend_cost_usd_micros(
                        &virtual_key_id,
                        &budget,
                        spent_cost_usd_micros,
                    );
                    if let Some((scope, budget)) = tenant_budget_scope.as_ref() {
                        gateway
                            .budget
                            .spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = project_budget_scope.as_ref() {
                        gateway
                            .budget
                            .spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                    if let Some((scope, budget)) = user_budget_scope.as_ref() {
                        gateway
                            .budget
                            .spend_cost_usd_micros(scope, budget, spent_cost_usd_micros);
                    }
                }
            }
        }

        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
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
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        if !_cost_budget_reserved && use_persistent_budget && spend_tokens {
            if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                (virtual_key_id.as_deref(), spent_cost_usd_micros)
            {
                #[cfg(feature = "gateway-store-sqlite")]
                if let Some(store) = state.sqlite_store.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
                #[cfg(feature = "gateway-store-redis")]
                if let Some(store) = state.redis_store.as_ref() {
                    let _ = store
                        .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                        .await;
                }
            }
        }

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        {
            let payload = serde_json::json!({
                "request_id": &request_id,
                "provider": &protocol,
                "virtual_key_id": virtual_key_id.as_deref(),
                "backend": &backend_name,
                "attempted_backends": &attempted_backends,
                "method": parts.method.as_str(),
                "path": path_and_query,
                "model": &model,
                "upstream_model": upstream_model.as_deref(),
                "service_tier": service_tier.as_deref(),
                "status": status.as_u16(),
                "charge_tokens": charge_tokens,
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
                "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
            });
            append_audit_log(state, "proxy", payload).await;
        }

        emit_json_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "provider": &protocol,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
                "model": &model,
                "upstream_model": upstream_model.as_deref(),
                "input_tokens": observed_usage.and_then(|usage| usage.input_tokens),
                "cache_input_tokens": observed_usage.and_then(|usage| usage.cache_input_tokens),
                "cache_creation_input_tokens": observed_usage.and_then(|usage| usage.cache_creation_input_tokens),
                "output_tokens": observed_usage.and_then(|usage| usage.output_tokens),
                "reasoning_tokens": observed_usage.and_then(|usage| usage.reasoning_tokens),
                "total_tokens": observed_usage.and_then(|usage| usage.total_tokens),
                "spent_tokens": spent_tokens,
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
            }),
        );

        #[cfg(feature = "gateway-otel")]
        {
            tracing::Span::current().record("cache", tracing::field::display("miss"));
            tracing::Span::current().record("backend", tracing::field::display(&backend_name));
            tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
        }

        #[cfg(feature = "gateway-proxy-cache")]
        if should_attempt_buffer_for_cache && status.is_success() {
            if let Some(cache_key) = proxy_cache_key.as_deref() {
                if let ProxyResponseBody::Bytes(bytes) = &response_body {
                    let cached = CachedProxyResponse {
                        status: status.as_u16(),
                        headers: upstream_headers.clone(),
                        body: bytes.clone(),
                        backend: backend_name.clone(),
                    };
                    store_proxy_cache_response(state, cache_key, cached, now_epoch_seconds()).await;
                }
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
        #[cfg(feature = "gateway-proxy-cache")]
        if let Some(cache_key) = proxy_cache_key.as_deref() {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }
        match response_body {
            ProxyResponseBody::Bytes(bytes) => {
                let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
                let mut response = axum::response::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
            ProxyResponseBody::Stream(stream) => {
                headers.remove("content-length");
                let stream = ProxyBodyStreamWithPermit {
                    inner: stream,
                    _permits: proxy_permits.take(),
                };
                let mut response = axum::response::Response::new(Body::from_stream(stream));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(BackendAttemptOutcome::Response(response))
            }
        }
}
