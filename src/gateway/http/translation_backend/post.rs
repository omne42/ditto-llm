            {
            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
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

            #[cfg(feature = "gateway-metrics-prometheus")]
            if let Some(metrics) = state.prometheus_metrics.as_ref() {
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
                metrics
                    .observe_proxy_request_duration(metrics_path, duration);
            }

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
                    "status": status.as_u16(),
                    "charge_tokens": charge_tokens,
                    "spent_tokens": spent_tokens,
                    "charge_cost_usd_micros": charge_cost_usd_micros,
                    "spent_cost_usd_micros": spent_cost_usd_micros,
                    "body_len": body.len(),
                    "mode": "translation",
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
                    "mode": "translation",
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
                        "mode": "translation",
                    }),
                );
            }

            Ok(BackendAttemptOutcome::Response(response))
            }
