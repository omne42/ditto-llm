use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{
    Gateway, GatewayError, GatewayRequest, GatewayResponse, ObservabilitySnapshot, VirtualKeyConfig,
};

#[derive(Clone)]
pub struct GatewayHttpState {
    gateway: Arc<Mutex<Gateway>>,
    admin_token: Option<String>,
}

impl GatewayHttpState {
    pub fn new(gateway: Gateway) -> Self {
        Self {
            gateway: Arc::new(Mutex::new(gateway)),
            admin_token: None,
        }
    }

    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Some(token.into());
        self
    }
}

#[derive(Debug, Deserialize)]
struct GatewayHttpRequest {
    #[serde(default)]
    virtual_key: Option<String>,
    model: String,
    prompt: String,
    input_tokens: u32,
    max_output_tokens: u32,
    #[serde(default)]
    passthrough: bool,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn router(state: GatewayHttpState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/gateway", post(handle_gateway))
        .route("/admin/keys", get(list_keys).post(upsert_key))
        .route(
            "/admin/keys/:id",
            put(upsert_key_with_id).delete(delete_key),
        )
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn metrics(State(state): State<GatewayHttpState>) -> Json<ObservabilitySnapshot> {
    let gateway = state.gateway.lock().await;
    Json(gateway.observability())
}

async fn handle_gateway(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(payload): Json<GatewayHttpRequest>,
) -> Result<Json<GatewayResponse>, (StatusCode, Json<ErrorResponse>)> {
    let virtual_key = payload
        .virtual_key
        .or_else(|| extract_bearer(&headers))
        .or_else(|| extract_header(&headers, "x-ditto-virtual-key"))
        .ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing virtual key",
            )
        })?;

    let request = GatewayRequest {
        virtual_key,
        model: payload.model,
        prompt: payload.prompt,
        input_tokens: payload.input_tokens,
        max_output_tokens: payload.max_output_tokens,
        passthrough: payload.passthrough,
    };

    let mut gateway = state.gateway.lock().await;
    match gateway.handle(request).await {
        Ok(response) => Ok(Json(response)),
        Err(err) => Err(map_gateway_error(err)),
    }
}

async fn list_keys(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<VirtualKeyConfig>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let gateway = state.gateway.lock().await;
    Ok(Json(gateway.list_virtual_keys()))
}

async fn upsert_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Json(key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let mut gateway = state.gateway.lock().await;
    let inserted = gateway.upsert_virtual_key(key.clone());
    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn upsert_key_with_id(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut key): Json<VirtualKeyConfig>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    key.id = id;
    let mut gateway = state.gateway.lock().await;
    let inserted = gateway.upsert_virtual_key(key.clone());
    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(key)))
}

async fn delete_key(
    State(state): State<GatewayHttpState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_admin(&state, &headers)?;
    let mut gateway = state.gateway.lock().await;
    if gateway.remove_virtual_key(&id).is_some() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "virtual key not found",
        ))
    }
}

fn ensure_admin(
    state: &GatewayHttpState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let Some(expected) = state.admin_token.as_deref() else {
        return Ok(());
    };
    let provided = extract_bearer(headers)
        .or_else(|| extract_header(headers, "x-admin-token"))
        .unwrap_or_default();
    if provided == expected {
        Ok(())
    } else {
        Err(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid admin token",
        ))
    }
}

fn extract_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())?;
    let trimmed = auth.trim();
    if let Some(rest) = trimmed.strip_prefix("Bearer ") {
        return Some(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("bearer ") {
        return Some(rest.trim().to_string());
    }
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    None
}

fn map_gateway_error(err: GatewayError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        GatewayError::Unauthorized => error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unauthorized virtual key",
        ),
        GatewayError::RateLimited { limit } => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => error_response(
            StatusCode::FORBIDDEN,
            "guardrail_rejected",
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => error_response(
            StatusCode::PAYMENT_REQUIRED,
            "budget_exceeded",
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::BackendNotFound { name } => error_response(
            StatusCode::BAD_GATEWAY,
            "backend_not_found",
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => {
            error_response(StatusCode::BAD_GATEWAY, "backend_error", message)
        }
        GatewayError::InvalidRequest { reason } => {
            error_response(StatusCode::BAD_REQUEST, "invalid_request", reason)
        }
    }
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
}
