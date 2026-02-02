fn validate_openai_multipart_request_schema(
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

    let parts = match super::multipart::parse_multipart_form(content_type, body) {
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
