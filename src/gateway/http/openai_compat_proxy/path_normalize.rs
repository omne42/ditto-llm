fn normalize_openai_compat_path(path: &str) -> std::borrow::Cow<'_, str> {
    if path.starts_with("/v1/") || path == "/v1" {
        return std::borrow::Cow::Borrowed(path);
    }

    match path {
        "/chat/completions" => std::borrow::Cow::Borrowed("/v1/chat/completions"),
        "/completions" => std::borrow::Cow::Borrowed("/v1/completions"),
        "/embeddings" => std::borrow::Cow::Borrowed("/v1/embeddings"),
        "/moderations" => std::borrow::Cow::Borrowed("/v1/moderations"),
        "/images/generations" => std::borrow::Cow::Borrowed("/v1/images/generations"),
        "/audio/transcriptions" => std::borrow::Cow::Borrowed("/v1/audio/transcriptions"),
        "/audio/translations" => std::borrow::Cow::Borrowed("/v1/audio/translations"),
        "/audio/speech" => std::borrow::Cow::Borrowed("/v1/audio/speech"),
        "/files" => std::borrow::Cow::Borrowed("/v1/files"),
        "/rerank" => std::borrow::Cow::Borrowed("/v1/rerank"),
        "/batches" => std::borrow::Cow::Borrowed("/v1/batches"),
        "/models" => std::borrow::Cow::Borrowed("/v1/models"),
        "/responses" => std::borrow::Cow::Borrowed("/v1/responses"),
        "/responses/compact" => std::borrow::Cow::Borrowed("/v1/responses/compact"),
        _ => {
            if path.starts_with("/models/")
                || path.starts_with("/files/")
                || path.starts_with("/batches/")
                || path.starts_with("/responses/")
            {
                return std::borrow::Cow::Owned(format!("/v1{path}"));
            }
            std::borrow::Cow::Borrowed(path)
        }
    }
}

fn normalize_openai_compat_path_and_query(path_and_query: &str) -> std::borrow::Cow<'_, str> {
    let Some((path, query)) = path_and_query.split_once('?') else {
        return normalize_openai_compat_path(path_and_query);
    };

    let normalized_path = normalize_openai_compat_path(path);
    if normalized_path.as_ref() == path {
        std::borrow::Cow::Borrowed(path_and_query)
    } else {
        std::borrow::Cow::Owned(format!("{}?{query}", normalized_path.as_ref()))
    }
}

async fn handle_openai_compat_proxy_root(
    State(state): State<GatewayHttpState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    handle_openai_compat_proxy(State(state), Path(String::new()), req).await
}

