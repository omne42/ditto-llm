async fn read_reqwest_body_bytes_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Bytes, std::io::Error> {
    let max_bytes = max_bytes.max(1);
    let mut stream = response.bytes_stream();
    let mut buffered = bytes::BytesMut::new();

    while let Some(next) = stream.next().await {
        match next {
            Ok(chunk) => {
                if buffered.len().saturating_add(chunk.len()) > max_bytes {
                    return Err(std::io::Error::other(format!(
                        "response exceeded max bytes ({max_bytes})"
                    )));
                }
                buffered.extend_from_slice(chunk.as_ref());
            }
            Err(err) => {
                return Err(std::io::Error::other(err));
            }
        }
    }

    Ok(buffered.freeze())
}

async fn read_reqwest_body_bytes_bounded_with_content_length(
    response: reqwest::Response,
    headers: &HeaderMap,
    max_bytes: usize,
) -> Result<Bytes, std::io::Error> {
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if content_length.is_some_and(|len| len > max_bytes) {
        return Err(std::io::Error::other(format!(
            "content-length={:?} exceeds max bytes ({max_bytes})",
            content_length
        )));
    }
    read_reqwest_body_bytes_bounded(response, max_bytes).await
}
