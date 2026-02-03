        {
        #[derive(Default)]
        struct SseUsageTracker {
            buffer: bytes::BytesMut,
            observed_usage: Option<ObservedUsage>,
        }

        impl SseUsageTracker {
            fn ingest(&mut self, chunk: &Bytes) {
                self.buffer.extend_from_slice(chunk.as_ref());

                loop {
                    let Some((pos, delimiter_len)) = find_sse_delimiter(self.buffer.as_ref())
                    else {
                        break;
                    };

                    let event_bytes = self.buffer.split_to(pos);
                    let _ = self.buffer.split_to(delimiter_len);

                    let Some(data) = extract_sse_data(event_bytes.as_ref()) else {
                        continue;
                    };
                    let trimmed = trim_ascii_whitespace(&data);
                    if trimmed == b"[DONE]" {
                        continue;
                    }

                    if trimmed.starts_with(b"{") {
                        let bytes = Bytes::copy_from_slice(trimmed);
                        if let Some(usage) = extract_openai_usage_from_bytes(&bytes) {
                            self.observed_usage = Some(usage);
                        }
                    }
                }
            }

            fn observed_usage(&self) -> Option<ObservedUsage> {
                self.observed_usage
            }
        }

        fn find_sse_delimiter(buf: &[u8]) -> Option<(usize, usize)> {
            if buf.len() >= 4 {
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    return Some((pos, 4));
                }
            }
            if buf.len() >= 2 {
                if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                    return Some((pos, 2));
                }
            }
            None
        }

        fn extract_sse_data(event: &[u8]) -> Option<Vec<u8>> {
            let mut out = Vec::<u8>::new();
            for line in event.split(|b| *b == b'\n') {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                let Some(rest) = line.strip_prefix(b"data:") else {
                    continue;
                };
                let rest = trim_ascii_whitespace(rest);
                if rest.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push(b'\n');
                }
                out.extend_from_slice(rest);
            }
            (!out.is_empty()).then_some(out)
        }

        fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
            let start = bytes
                .iter()
                .position(|b| !b.is_ascii_whitespace())
                .unwrap_or(bytes.len());
            let end = bytes
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .map(|pos| pos + 1)
                .unwrap_or(start);
            &bytes[start..end]
        }

        struct ProxySseFinalizer {
            state: GatewayHttpState,
            backend_name: String,
            attempted_backends: Vec<String>,
            request_id: String,
            method: String,
            path_and_query: String,
            model: Option<String>,
            service_tier: Option<String>,
            backend_model_map: BTreeMap<String, String>,
            status: u16,
            charge_tokens: u32,
            charge_cost_usd_micros: Option<u64>,
            spend_tokens: bool,
            use_persistent_budget: bool,
            virtual_key_id: Option<String>,
            budget: Option<super::BudgetConfig>,
            tenant_budget_scope: Option<(String, super::BudgetConfig)>,
            project_budget_scope: Option<(String, super::BudgetConfig)>,
            user_budget_scope: Option<(String, super::BudgetConfig)>,
            token_budget_reservation_ids: Vec<String>,
            cost_budget_reserved: bool,
            cost_budget_reservation_ids: Vec<String>,
            request_body_len: usize,
        }

        impl ProxySseFinalizer {
            async fn finalize(self, observed_usage: Option<ObservedUsage>) {
                let spent_tokens = if self.spend_tokens {
                    observed_usage
                        .and_then(|usage| usage.total_tokens)
                        .unwrap_or(u64::from(self.charge_tokens))
                } else {
                    0
                };

                #[cfg(feature = "gateway-costing")]
                let spent_cost_usd_micros = if self.spend_tokens {
                    self.model
                        .as_deref()
                        .map(|request_model| {
                            self.backend_model_map
                                .get(request_model)
                                .map(|model| model.as_str())
                                .unwrap_or(request_model)
                        })
                        .and_then(|cost_model| {
                            self.state.pricing.as_ref().and_then(|pricing| {
                                let usage = observed_usage?;
                                let input = usage.input_tokens?;
                                let output = usage.output_tokens?;
                                pricing.estimate_cost_usd_micros_with_cache_for_service_tier(
                                    cost_model,
                                    clamp_u64_to_u32(input),
                                    usage.cache_input_tokens.map(clamp_u64_to_u32),
                                    usage.cache_creation_input_tokens.map(clamp_u64_to_u32),
                                    clamp_u64_to_u32(output),
                                    self.service_tier.as_deref(),
                                )
                            })
                        })
                        .or(self.charge_cost_usd_micros)
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

                #[cfg(not(any(
                    feature = "gateway-costing",
                    feature = "gateway-store-sqlite",
                    feature = "gateway-store-redis",
                    feature = "sdk",
                )))]
                let _ = (
                    &self.method,
                    &self.path_and_query,
                    &self.model,
                    &self.service_tier,
                    &self.backend_model_map,
                    self.charge_cost_usd_micros,
                    self.use_persistent_budget,
                    &self.token_budget_reservation_ids,
                    self.cost_budget_reserved,
                    &self.cost_budget_reservation_ids,
                    self.request_body_len,
                );

                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
                if !self.token_budget_reservation_ids.is_empty() {
                    settle_proxy_token_budget_reservations(
                        &self.state,
                        &self.token_budget_reservation_ids,
                        self.spend_tokens,
                        spent_tokens,
                    )
                    .await;
                } else if let (Some(virtual_key_id), Some(budget)) =
                    (self.virtual_key_id.clone(), self.budget.clone())
                {
                    if self.spend_tokens {
                        let mut gateway = self.state.gateway.lock().await;
                        gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                        if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }
                        if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }
                        if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }

                        #[cfg(feature = "gateway-costing")]
                        if !self.use_persistent_budget {
                            if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                                gateway.budget.spend_cost_usd_micros(
                                    &virtual_key_id,
                                    &budget,
                                    spent_cost_usd_micros,
                                );
                                if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                    gateway.budget.spend_cost_usd_micros(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                    gateway.budget.spend_cost_usd_micros(
                                        scope,
                                        budget,
                                        spent_cost_usd_micros,
                                    );
                                }
                                if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
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
                if let (Some(virtual_key_id), Some(budget)) =
                    (self.virtual_key_id.clone(), self.budget.clone())
                {
                    if self.spend_tokens {
                        let mut gateway = self.state.gateway.lock().await;
                        gateway.budget.spend(&virtual_key_id, &budget, spent_tokens);
                        if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }
                        if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }
                        if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
                            gateway.budget.spend(scope, budget, spent_tokens);
                        }

                        #[cfg(feature = "gateway-costing")]
                        if let Some(spent_cost_usd_micros) = spent_cost_usd_micros {
                            gateway.budget.spend_cost_usd_micros(
                                &virtual_key_id,
                                &budget,
                                spent_cost_usd_micros,
                            );
                            if let Some((scope, budget)) = self.tenant_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    spent_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = self.project_budget_scope.as_ref() {
                                gateway.budget.spend_cost_usd_micros(
                                    scope,
                                    budget,
                                    spent_cost_usd_micros,
                                );
                            }
                            if let Some((scope, budget)) = self.user_budget_scope.as_ref() {
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
                if !self.cost_budget_reservation_ids.is_empty() {
                    settle_proxy_cost_budget_reservations(
                        &self.state,
                        &self.cost_budget_reservation_ids,
                        self.spend_tokens,
                        spent_cost_usd_micros.unwrap_or_default(),
                    )
                    .await;
                }

                #[cfg(all(
                    feature = "gateway-costing",
                    any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
                ))]
                if !self.cost_budget_reserved && self.use_persistent_budget && self.spend_tokens {
                    if let (Some(virtual_key_id), Some(spent_cost_usd_micros)) =
                        (self.virtual_key_id.as_deref(), spent_cost_usd_micros)
                    {
                        #[cfg(feature = "gateway-store-sqlite")]
                        if let Some(store) = self.state.sqlite_store.as_ref() {
                            let _ = store
                                .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                                .await;
                        }
                        #[cfg(feature = "gateway-store-redis")]
                        if let Some(store) = self.state.redis_store.as_ref() {
                            let _ = store
                                .record_spent_cost_usd_micros(virtual_key_id, spent_cost_usd_micros)
                                .await;
                        }
                    }
                }

                #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
                {
                    let payload = serde_json::json!({
                        "request_id": &self.request_id,
                        "virtual_key_id": self.virtual_key_id.as_deref(),
                        "backend": &self.backend_name,
                        "attempted_backends": &self.attempted_backends,
                        "method": &self.method,
                        "path": &self.path_and_query,
                        "model": &self.model,
                        "status": self.status,
                        "charge_tokens": self.charge_tokens,
                        "spent_tokens": spent_tokens,
                        "charge_cost_usd_micros": self.charge_cost_usd_micros,
                        "spent_cost_usd_micros": spent_cost_usd_micros,
                        "body_len": self.request_body_len,
                        "stream": true,
                    });

                    #[cfg(feature = "gateway-store-sqlite")]
                    if let Some(store) = self.state.sqlite_store.as_ref() {
                        let _ = store.append_audit_log("proxy", payload.clone()).await;
                    }
                    #[cfg(feature = "gateway-store-redis")]
                    if let Some(store) = self.state.redis_store.as_ref() {
                        let _ = store.append_audit_log("proxy", payload.clone()).await;
                    }
                }

                emit_json_log(
                    &self.state,
                    "proxy.response",
                    serde_json::json!({
                        "request_id": &self.request_id,
                        "backend": &self.backend_name,
                        "status": self.status,
                        "attempted_backends": &self.attempted_backends,
                        "spent_tokens": spent_tokens,
                    }),
                );

                #[cfg(feature = "sdk")]
                if let Some(logger) = self.state.devtools.as_ref() {
                    let _ = logger.log_event(
                        "proxy.response",
                        serde_json::json!({
                            "request_id": &self.request_id,
                            "status": self.status,
                            "path": &self.path_and_query,
                            "backend": &self.backend_name,
                            "spent_tokens": spent_tokens,
                        }),
                    );
                }
            }
        }

        struct ProxySseStreamState {
            upstream: ProxyBodyStream,
            tracker: SseUsageTracker,
            finalizer: Option<ProxySseFinalizer>,
            _permits: ProxyPermits,
        }

        impl Drop for ProxySseStreamState {
            fn drop(&mut self) {
                let Some(finalizer) = self.finalizer.take() else {
                    return;
                };
                let observed = self.tracker.observed_usage();
                tokio::spawn(async move { finalizer.finalize(observed).await });
            }
        }

        impl ProxySseStreamState {
            async fn finalize(&mut self) {
                let Some(finalizer) = self.finalizer.take() else {
                    return;
                };
                let observed = self.tracker.observed_usage();
                finalizer.finalize(observed).await;
            }
        }

        let mut headers = upstream_headers;
        apply_proxy_response_headers(&mut headers, &backend_name, &request_id, false);
        #[cfg(feature = "gateway-proxy-cache")]
        if let Some(cache_key) = proxy_cache_key.as_ref() {
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_key) {
                headers.insert("x-ditto-cache-key", value);
            }
        }

        #[cfg(feature = "gateway-otel")]
        {
            tracing::Span::current().record("cache", tracing::field::display("miss"));
            tracing::Span::current().record("backend", tracing::field::display(&backend_name));
            tracing::Span::current().record("status", tracing::field::display(status.as_u16()));
        }

        let upstream_stream: ProxyBodyStream = upstream_response
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();

        #[cfg(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"))]
        let token_budget_reservation_ids = token_budget_reservation_ids.to_vec();
        #[cfg(not(any(feature = "gateway-store-sqlite", feature = "gateway-store-redis")))]
        let token_budget_reservation_ids = Vec::new();

        #[cfg(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        ))]
        let cost_budget_reservation_ids = cost_budget_reservation_ids.to_vec();
        #[cfg(not(all(
            feature = "gateway-costing",
            any(feature = "gateway-store-sqlite", feature = "gateway-store-redis"),
        )))]
        let cost_budget_reservation_ids = Vec::new();

        let finalizer = ProxySseFinalizer {
            state: state.to_owned(),
            backend_name: backend_name.clone(),
            attempted_backends: attempted_backends.to_vec(),
            request_id: request_id.clone(),
            method: parts.method.as_str().to_string(),
            path_and_query: path_and_query.to_string(),
            model: model.to_owned(),
            service_tier: service_tier.to_owned(),
            backend_model_map: backend_model_map.clone(),
            status: status.as_u16(),
            charge_tokens,
            charge_cost_usd_micros,
            spend_tokens,
            use_persistent_budget,
            virtual_key_id: virtual_key_id.to_owned(),
            budget: budget.to_owned(),
            tenant_budget_scope: tenant_budget_scope.to_owned(),
            project_budget_scope: project_budget_scope.to_owned(),
            user_budget_scope: user_budget_scope.to_owned(),
            token_budget_reservation_ids,
            cost_budget_reserved: _cost_budget_reserved,
            cost_budget_reservation_ids,
            request_body_len: body.len(),
        };

        let state = ProxySseStreamState {
            upstream: upstream_stream,
            tracker: SseUsageTracker::default(),
            finalizer: Some(finalizer),
            _permits: proxy_permits.take(),
        };

        let stream = futures_util::stream::try_unfold(state, |mut state| async move {
            match state.upstream.next().await {
                Some(Ok(chunk)) => {
                    state.tracker.ingest(&chunk);
                    Ok(Some((chunk, state)))
                }
                Some(Err(err)) => {
                    state.finalize().await;
                    Err(err)
                }
                None => {
                    state.finalize().await;
                    Ok(None)
                }
            }
        });

        let mut response = axum::response::Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        return Ok(BackendAttemptOutcome::Response(response));
        }
