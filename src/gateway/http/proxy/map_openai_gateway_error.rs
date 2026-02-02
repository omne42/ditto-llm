fn map_openai_gateway_error(err: GatewayError) -> (StatusCode, Json<OpenAiErrorResponse>) {
    match err {
        GatewayError::Unauthorized => openai_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            Some("invalid_api_key"),
            "unauthorized virtual key",
        ),
        GatewayError::RateLimited { limit } => openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            Some("rate_limited"),
            format!("rate limit exceeded: {limit}"),
        ),
        GatewayError::GuardrailRejected { reason } => openai_error(
            StatusCode::FORBIDDEN,
            "policy_error",
            Some("guardrail_rejected"),
            format!("guardrail rejected: {reason}"),
        ),
        GatewayError::BudgetExceeded { limit, attempted } => openai_error(
            StatusCode::PAYMENT_REQUIRED,
            "insufficient_quota",
            Some("budget_exceeded"),
            format!("budget exceeded: limit={limit} attempted={attempted}"),
        ),
        GatewayError::CostBudgetExceeded {
            limit_usd_micros,
            attempted_usd_micros,
        } => openai_error(
            StatusCode::PAYMENT_REQUIRED,
            "insufficient_quota",
            Some("cost_budget_exceeded"),
            format!(
                "cost budget exceeded: limit_usd_micros={limit_usd_micros} attempted_usd_micros={attempted_usd_micros}"
            ),
        ),
        GatewayError::BackendNotFound { name } => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_not_found"),
            format!("backend not found: {name}"),
        ),
        GatewayError::Backend { message } => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            Some("backend_error"),
            message,
        ),
        GatewayError::InvalidRequest { reason } => openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            Some("invalid_request"),
            reason,
        ),
    }
}

