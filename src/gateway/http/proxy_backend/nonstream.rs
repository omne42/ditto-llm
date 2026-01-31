{
        let bytes = upstream_response.bytes().await.unwrap_or_default();
        let observed_usage = if spend_tokens && content_type.starts_with("application/json") {
            extract_openai_usage_from_bytes(&bytes)
        } else {
            None
        };

        let spent_tokens = if spend_tokens {
            observed_usage
                .and_then(|usage| usage.total_tokens)
                .unwrap_or(u64::from(charge_tokens))
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
                    "virtual_key_id": virtual_key_id.as_deref(),
                    "backend": &backend_name,
                    "attempted_backends": &attempted_backends,
                    "method": parts.method.as_str(),
                    "path": path_and_query,
                    "model": &model,
                    "service_tier": service_tier.as_deref(),
                    "status": status.as_u16(),
                    "charge_tokens": charge_tokens,
                    "spent_tokens": spent_tokens,
                    "charge_cost_usd_micros": charge_cost_usd_micros,
                "spent_cost_usd_micros": spent_cost_usd_micros,
                "body_len": body.len(),
            });

            #[cfg(feature = "gateway-store-sqlite")]
            if let Some(store) = state.sqlite_store.as_ref() {
                let _ = store.append_audit_log("proxy", payload.clone()).await;
            }
            #[cfg(feature = "gateway-store-redis")]
            if let Some(store) = state.redis_store.as_ref() {
                let _ = store.append_audit_log("proxy", payload.clone()).await;
            }
        }

        emit_json_log(
            state,
            "proxy.response",
            serde_json::json!({
                "request_id": &request_id,
                "backend": &backend_name,
                "status": status.as_u16(),
                "attempted_backends": &attempted_backends,
            }),
        );

        #[cfg(feature = "sdk")]
        if let Some(logger) = state.devtools.as_ref() {
            let _ = logger.log_event(
                "proxy.response",
                serde_json::json!({
                    "request_id": &request_id,
                    "status": status.as_u16(),
                    "path": path_and_query,
                    "backend": &backend_name,
                }),
            );
        }

        #[cfg(feature = "gateway-otel")]
        {
            tracing::Span::current().record("cache", tracing::field::display("miss"));
            tracing::Span::current().record("backend", tracing::field::display(&backend_name));
            tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
        }

        #[cfg(feature = "gateway-proxy-cache")]
        if status.is_success() {
            if let Some(cache_key) = proxy_cache_key.as_deref() {
                let cached = CachedProxyResponse {
                    status: status.as_u16(),
                    headers: upstream_headers.clone(),
                    body: bytes.clone(),
                    backend: backend_name.clone(),
                };
                store_proxy_cache_response(state, cache_key, cached, now_epoch_seconds()).await;
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
        let body = proxy_body_from_bytes_with_permit(bytes, proxy_permits.take());
        let mut response = axum::response::Response::new(body);
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Ok(BackendAttemptOutcome::Response(response))
}
