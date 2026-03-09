use super::*;

pub(super) fn extract_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn extract_query_param(uri: &axum::http::Uri, name: &str) -> Option<String> {
    let query = uri.query()?;
    extract_query_param_str(query, name)
}

fn extract_query_param_str(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != name {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        return percent_decode_www_form(value);
    }
    None
}

fn percent_decode_www_form(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = from_hex(bytes[i + 1])?;
                let lo = from_hex(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())?
        .trim()
        .to_string();
    let rest = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    let token = rest.trim();
    (!token.is_empty()).then(|| token.to_string())
}

pub(super) fn extract_litellm_api_key(headers: &HeaderMap) -> Option<String> {
    let raw = extract_header(headers, "x-litellm-api-key")?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw.as_str())
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

pub(super) fn extract_virtual_key(headers: &HeaderMap) -> Option<String> {
    extract_litellm_api_key(headers)
        .or_else(|| extract_bearer(headers))
        .or_else(|| extract_header(headers, "x-ditto-virtual-key"))
        .or_else(|| extract_header(headers, "x-api-key"))
}
