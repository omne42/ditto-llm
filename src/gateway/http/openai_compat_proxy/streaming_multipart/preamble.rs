fn should_stream_large_multipart_request(
    parts: &axum::http::request::Parts,
    path_and_query: &str,
    max_body_bytes: usize,
) -> bool {
    if parts.method != axum::http::Method::POST {
        return false;
    }

    let path = path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
        .trim_end_matches('/');
    if path != "/v1/files" && path != "/v1/audio/transcriptions" && path != "/v1/audio/translations"
    {
        return false;
    }

    let is_multipart = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("multipart/form-data"));
    if !is_multipart {
        return false;
    }

    let content_length = parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<usize>().ok());
    content_length.is_some_and(|len| len > max_body_bytes)
}

fn estimate_tokens_from_length(len: usize) -> u32 {
    if len == 0 {
        return 0;
    }
    let estimate = (len.saturating_add(3) / 4) as u64;
    if estimate > u64::from(u32::MAX) {
        u32::MAX
    } else {
        estimate as u32
    }
}

