fn parse_json_from_response_text(text: &str) -> Result<(Value, Option<Warning>)> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err(DittoError::InvalidResponse(
            "model returned an empty response; expected JSON".to_string(),
        ));
    }

    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        return Ok((parsed, None));
    }

    if let Some(block) = extract_code_fence(raw) {
        if let Ok(parsed) = serde_json::from_str::<Value>(block.trim()) {
            return Ok((
                parsed,
                Some(Warning::Compatibility {
                    feature: "object.json_extraction".to_string(),
                    details: "extracted JSON from a fenced code block".to_string(),
                }),
            ));
        }
    }

    if let Some(substring) = extract_balanced_json(raw) {
        if let Ok(parsed) = serde_json::from_str::<Value>(substring.trim()) {
            return Ok((
                parsed,
                Some(Warning::Compatibility {
                    feature: "object.json_extraction".to_string(),
                    details: "extracted JSON from a larger text response".to_string(),
                }),
            ));
        }
    }

    Err(DittoError::InvalidResponse(format!(
        "failed to parse model response as JSON (response starts with {:?})",
        raw.chars().take(120).collect::<String>()
    )))
}

fn extract_code_fence(text: &str) -> Option<String> {
    let start = text.find("```")?;
    let after_start = &text[start + 3..];
    let start_content_rel = after_start.find('\n').map(|idx| idx + 1)?;
    let start_content = start + 3 + start_content_rel;

    let remaining = &text[start_content..];
    let end_rel = remaining.find("```")?;
    let end = start_content + end_rel;
    let block = text[start_content..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_string())
    }
}

fn extract_balanced_json(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut last_end: Option<usize> = None;

    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&b) {
                    stack.pop();
                    if stack.is_empty() {
                        last_end = Some(start + offset + 1);
                    }
                }
            }
            _ => {}
        }
    }

    last_end.map(|end| &text[start..end])
}

fn parse_partial_json(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut stack: Vec<u8> = Vec::new();
    let mut last_complete_end: Option<usize> = None;

    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match b {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&b) {
                    stack.pop();
                    if stack.is_empty() {
                        last_complete_end = Some(start + offset + 1);
                    }
                }
            }
            _ => {}
        }
    }

    if in_string || escape {
        return None;
    }

    if let Some(end) = last_complete_end {
        return serde_json::from_str::<Value>(text[start..end].trim()).ok();
    }

    let mut candidate = text[start..].to_string();

    loop {
        let trimmed = candidate.trim_end();
        let Some(last) = trimmed.as_bytes().last().copied() else {
            break;
        };
        if last == b',' || last == b':' {
            candidate.truncate(trimmed.len().saturating_sub(1));
            continue;
        }
        break;
    }

    for &closing in stack.iter().rev() {
        candidate.push(closing as char);
    }

    serde_json::from_str::<Value>(candidate.trim()).ok()
}
