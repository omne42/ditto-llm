#[cfg(feature = "gateway-costing")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CostBudgetEndpointPolicy {
    TokenBased,
    Free,
    Unsupported,
}

#[cfg(feature = "gateway-costing")]
fn cost_budget_endpoint_policy(
    method: &axum::http::Method,
    path_and_query: &str,
) -> CostBudgetEndpointPolicy {
    if *method != axum::http::Method::POST {
        return CostBudgetEndpointPolicy::Free;
    }

    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');

    if path == "/v1/chat/completions"
        || path == "/v1/completions"
        || path == "/v1/embeddings"
        || path == "/v1/moderations"
        || path == "/v1/rerank"
        || path.starts_with("/v1/responses")
    {
        return CostBudgetEndpointPolicy::TokenBased;
    }

    if path == "/v1/files" {
        return CostBudgetEndpointPolicy::Free;
    }

    CostBudgetEndpointPolicy::Unsupported
}

