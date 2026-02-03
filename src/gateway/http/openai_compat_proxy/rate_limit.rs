#[cfg(feature = "gateway-store-redis")]
fn normalize_rate_limit_route(path_and_query: &str) -> String {
    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query);
    let path = path.strip_suffix('/').unwrap_or(path);

    match path {
        "/v1/chat/completions"
        | "/v1/completions"
        | "/v1/embeddings"
        | "/v1/moderations"
        | "/v1/images/generations"
        | "/v1/audio/transcriptions"
        | "/v1/audio/translations"
        | "/v1/audio/speech"
        | "/v1/files"
        | "/v1/rerank"
        | "/v1/batches"
        | "/v1/models"
        | "/v1/responses"
        | "/v1/responses/compact" => path.to_string(),
        _ => {
            if path.starts_with("/v1/models/") {
                return "/v1/models/*".to_string();
            }
            if path.starts_with("/v1/batches/") {
                if path.ends_with("/cancel") {
                    return "/v1/batches/*/cancel".to_string();
                }
                return "/v1/batches/*".to_string();
            }
            if path.starts_with("/v1/files/") {
                if path.ends_with("/content") {
                    return "/v1/files/*/content".to_string();
                }
                return "/v1/files/*".to_string();
            }
            if path.starts_with("/v1/responses/") {
                return "/v1/responses/*".to_string();
            }

            "/v1/*".to_string()
        }
    }
}
