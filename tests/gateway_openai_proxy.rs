#![cfg(feature = "gateway")]

use std::collections::{BTreeMap, HashMap};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use ditto_llm::gateway::{
    BackendConfig, BudgetConfig, Gateway, GatewayConfig, GatewayHttpState, GuardrailsConfig,
    ProxyBackend, RouteRule, RouterConfig, VirtualKeyConfig,
};
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;
use tower::util::ServiceExt;

fn build_proxy_backends(
    config: &GatewayConfig,
) -> Result<HashMap<String, ProxyBackend>, ditto_llm::gateway::GatewayError> {
    let mut out = HashMap::new();
    for backend in &config.backends {
        let mut client = ProxyBackend::new(&backend.base_url)?;
        client = client.with_headers(backend.headers.clone())?;
        client = client.with_query_params(backend.query_params.clone());
        client = client.with_request_timeout_seconds(backend.timeout_seconds);
        out.insert(backend.name.clone(), client);
    }
    Ok(out)
}

fn backend_config(name: &str, base_url: String, auth: &str) -> BackendConfig {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_string(), auth.to_string());
    BackendConfig {
        name: name.to_string(),
        base_url,
        max_in_flight: None,
        timeout_seconds: None,
        headers,
        query_params: BTreeMap::new(),
        provider: None,
        provider_config: None,
        model_map: BTreeMap::new(),
    }
}

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("gateway_openai_proxy/part01.rs");
include!("gateway_openai_proxy/part02.rs");
include!("gateway_openai_proxy/part03.rs");
