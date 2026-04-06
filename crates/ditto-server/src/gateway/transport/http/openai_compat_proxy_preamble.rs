use super::*;

#[derive(Clone, Copy)]
pub(super) struct ProxyAttemptParams<'a> {
    pub(super) state: &'a GatewayHttpState,
    pub(super) parts: &'a axum::http::request::Parts,
    pub(super) body: &'a Bytes,
    pub(super) parsed_json: &'a Option<serde_json::Value>,
    pub(super) model: &'a Option<String>,
    pub(super) service_tier: &'a Option<String>,
    pub(super) request_id: &'a str,
    #[cfg(feature = "gateway-routing-advanced")]
    pub(super) client_supplied_request_id: bool,
    pub(super) path_and_query: &'a str,
    pub(super) now_epoch_seconds: u64,
    pub(super) charge_tokens: u32,
    pub(super) stream_requested: bool,
    pub(super) strip_authorization: bool,
    pub(super) use_persistent_budget: bool,
    pub(super) virtual_key_id: &'a Option<String>,
    #[cfg(feature = "gateway-translation")]
    pub(super) response_owner: &'a super::translation::TranslationResponseOwner,
    pub(super) budget: &'a Option<super::BudgetConfig>,
    pub(super) tenant_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) project_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) user_budget_scope: &'a Option<(String, super::BudgetConfig)>,
    pub(super) local_token_budget_reserved: bool,
    #[cfg(feature = "gateway-costing")]
    pub(super) local_cost_budget_reserved: bool,
    pub(super) charge_cost_usd_micros: Option<u64>,
    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    pub(super) token_budget_reservation_ids: &'a [String],
    pub(super) cost_budget_reserved: bool,
    #[cfg(all(
        feature = "gateway-costing",
        any(
            feature = "gateway-store-sqlite",
            feature = "gateway-store-postgres",
            feature = "gateway-store-mysql",
            feature = "gateway-store-redis"
        ),
    ))]
    pub(super) cost_budget_reservation_ids: &'a [String],
    pub(super) max_attempts: usize,
    #[cfg(feature = "gateway-routing-advanced")]
    pub(super) retry_config: &'a super::super::ProxyRetryConfig,
    #[cfg(feature = "gateway-proxy-cache")]
    pub(super) proxy_cache_key: &'a Option<String>,
    #[cfg(feature = "gateway-proxy-cache")]
    pub(super) proxy_cache_metadata: &'a Option<ProxyCacheEntryMetadata>,
    #[cfg(feature = "gateway-metrics-prometheus")]
    pub(super) metrics_path: &'a str,
    #[cfg(feature = "gateway-metrics-prometheus")]
    pub(super) metrics_timer_start: Instant,
}

pub(super) enum BackendAttemptOutcome {
    Response(axum::response::Response),
    Continue(Option<(StatusCode, Json<OpenAiErrorResponse>)>),
    #[allow(dead_code)]
    Stop((StatusCode, Json<OpenAiErrorResponse>)),
}

// inlined from multipart_schema.rs
pub(super) fn validate_openai_multipart_request_schema(
    path_and_query: &str,
    content_type: Option<&str>,
    body: &Bytes,
) -> Option<String> {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');

    let endpoint = if path == "/v1/audio/transcriptions" {
        "audio/transcriptions"
    } else if path == "/v1/audio/translations" {
        "audio/translations"
    } else if path == "/v1/files" {
        "files"
    } else {
        return None;
    };

    let Some(content_type) = content_type else {
        return Some(format!("{endpoint} request missing content-type"));
    };
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Some(format!("{endpoint} request must be multipart/form-data"));
    }

    let parts = match super::super::multipart::parse_multipart_form(content_type, body) {
        Ok(parts) => parts,
        Err(err) => return Some(err),
    };

    if endpoint.starts_with("audio/") {
        let mut has_file = false;
        let mut has_model = false;
        for part in parts {
            match part.name.as_str() {
                "file" => has_file = true,
                "model" if part.filename.is_none() => {
                    let value = String::from_utf8_lossy(part.data.as_ref())
                        .trim()
                        .to_string();
                    if !value.is_empty() {
                        has_model = true;
                    }
                }
                _ => {}
            }
        }

        if !has_file {
            return Some(format!("{endpoint} request missing file"));
        }
        if !has_model {
            return Some(format!("{endpoint} request missing model"));
        }
        return None;
    }

    let mut has_file = false;
    let mut has_purpose = false;
    for part in parts {
        match part.name.as_str() {
            "file" => has_file = true,
            "purpose" if part.filename.is_none() => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    has_purpose = true;
                }
            }
            _ => {}
        }
    }

    if !has_file {
        return Some("files request missing file".to_string());
    }
    if !has_purpose {
        return Some("files request missing purpose".to_string());
    }
    None
}
// end inline: multipart_schema.rs
