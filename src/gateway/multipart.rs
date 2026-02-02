use bytes::Bytes;

#[derive(Debug, Clone)]
pub(crate) struct MultipartPart {
    pub(crate) name: String,
    pub(crate) filename: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) data: Bytes,
}

fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    if start >= haystack.len() {
        return None;
    }
    let first = needle[0];
    let mut pos = start;
    while pos + needle.len() <= haystack.len() {
        let rel = haystack[pos..].iter().position(|&b| b == first)?;
        pos += rel;
        if pos + needle.len() > haystack.len() {
            return None;
        }
        if &haystack[pos..pos + needle.len()] == needle {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn multipart_boundary(content_type: &str) -> Result<String, String> {
    for part in content_type.split(';').map(str::trim) {
        if part.len() < "boundary=".len() {
            continue;
        }
        if !part[..].to_ascii_lowercase().starts_with("boundary=") {
            continue;
        }

        let value = part["boundary=".len()..].trim();
        if value.is_empty() {
            continue;
        }

        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);

        if unquoted.trim().is_empty() {
            continue;
        }

        return Ok(unquoted.to_string());
    }

    Err("multipart boundary is missing".to_string())
}

pub(crate) fn parse_multipart_form(
    content_type: &str,
    body: &Bytes,
) -> Result<Vec<MultipartPart>, String> {
    let boundary = multipart_boundary(content_type)?;
    let boundary_marker = format!("--{boundary}");
    let boundary_bytes = boundary_marker.as_bytes();
    let delimiter = format!("\r\n{boundary_marker}");
    let delimiter_bytes = delimiter.as_bytes();

    let bytes = body.as_ref();
    let Some(mut cursor) = find_subslice(bytes, boundary_bytes, 0) else {
        return Err("multipart body missing boundary marker".to_string());
    };
    cursor += boundary_bytes.len();

    let mut parts = Vec::<MultipartPart>::new();
    loop {
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }

        let (headers_end, header_sep_len) =
            if let Some(idx) = find_subslice(bytes, b"\r\n\r\n", cursor) {
                (idx, 4)
            } else if let Some(idx) = find_subslice(bytes, b"\n\n", cursor) {
                (idx, 2)
            } else {
                return Err("multipart part missing header separator".to_string());
            };

        let headers_raw = String::from_utf8_lossy(&bytes[cursor..headers_end]);
        let mut name: Option<String> = None;
        let mut filename: Option<String> = None;
        let mut content_type: Option<String> = None;

        for line in headers_raw.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("content-disposition") {
                for item in value.split(';').map(str::trim) {
                    if let Some(value) = item.strip_prefix("name=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        name = Some(value.to_string());
                    } else if let Some(value) = item.strip_prefix("filename=") {
                        let value = value.trim();
                        let value = value
                            .strip_prefix('"')
                            .and_then(|v| v.strip_suffix('"'))
                            .unwrap_or(value);
                        filename = Some(value.to_string());
                    }
                }
            } else if key.eq_ignore_ascii_case("content-type") && !value.is_empty() {
                content_type = Some(value.to_string());
            }
        }

        let name =
            name.ok_or_else(|| "multipart part missing content-disposition name".to_string())?;
        let data_start = headers_end + header_sep_len;

        let Some(delim_pos) = find_subslice(bytes, delimiter_bytes, data_start) else {
            return Err("multipart part missing trailing boundary".to_string());
        };
        let data_end = delim_pos;

        let data = body.slice(data_start..data_end);
        parts.push(MultipartPart {
            name,
            filename,
            content_type,
            data,
        });

        cursor = delim_pos + delimiter_bytes.len();
        if bytes.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if bytes.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }
    }

    Ok(parts)
}
