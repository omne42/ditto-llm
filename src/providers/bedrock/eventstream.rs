#[derive(Debug)]
struct EventStreamMessage {
    headers: HashMap<String, String>,
    payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct EventStreamDecoder {
    buffer: Vec<u8>,
}

impl EventStreamDecoder {
    fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    fn next_message(&mut self) -> Option<Result<EventStreamMessage>> {
        if self.buffer.len() < 12 {
            return None;
        }
        let total_len = u32::from_be_bytes(self.buffer[0..4].try_into().ok()?) as usize;
        if total_len < 16 {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream total_len too small".to_string(),
            )));
        }
        if self.buffer.len() < total_len {
            return None;
        }
        let message = self.buffer.drain(0..total_len).collect::<Vec<u8>>();
        let headers_len = u32::from_be_bytes(message[4..8].try_into().ok()?) as usize;
        let headers_start = 12usize;
        let headers_end = headers_start.saturating_add(headers_len);
        if headers_end > message.len() {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream invalid headers length".to_string(),
            )));
        }
        let payload_end = total_len.saturating_sub(4);
        if headers_end > payload_end {
            return Some(Err(DittoError::InvalidResponse(
                "eventstream invalid payload length".to_string(),
            )));
        }

        let headers = match parse_eventstream_headers(&message[headers_start..headers_end]) {
            Ok(headers) => headers,
            Err(err) => return Some(Err(err)),
        };
        let payload = message[headers_end..payload_end].to_vec();
        Some(Ok(EventStreamMessage { headers, payload }))
    }
}

fn parse_eventstream_headers(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::<String, String>::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let name_len = *bytes.get(idx).ok_or_else(|| {
            DittoError::InvalidResponse("eventstream header missing name length".to_string())
        })? as usize;
        idx += 1;
        if idx + name_len > bytes.len() {
            return Err(DittoError::InvalidResponse(
                "eventstream header name truncated".to_string(),
            ));
        }
        let name = std::str::from_utf8(&bytes[idx..idx + name_len]).map_err(|err| {
            DittoError::InvalidResponse(format!("eventstream bad header name: {err}"))
        })?;
        idx += name_len;
        let value_type = *bytes.get(idx).ok_or_else(|| {
            DittoError::InvalidResponse("eventstream header missing type".to_string())
        })?;
        idx += 1;
        let ensure_len = |idx: usize, needed: usize, label: &str| -> Result<()> {
            if idx + needed > bytes.len() {
                return Err(DittoError::InvalidResponse(format!(
                    "eventstream header value truncated ({label})"
                )));
            }
            Ok(())
        };

        match value_type {
            0 | 1 => {}
            2 => {
                ensure_len(idx, 1, "byte")?;
                idx += 1;
            }
            3 => {
                ensure_len(idx, 2, "short")?;
                idx += 2;
            }
            4 => {
                ensure_len(idx, 4, "int")?;
                idx += 4;
            }
            5 => {
                ensure_len(idx, 8, "long")?;
                idx += 8;
            }
            6 | 7 => {
                ensure_len(idx, 2, "length")?;
                let len = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]) as usize;
                idx += 2;
                ensure_len(idx, len, "bytes")?;
                if value_type == 7 {
                    let value = std::str::from_utf8(&bytes[idx..idx + len]).map_err(|err| {
                        DittoError::InvalidResponse(format!(
                            "eventstream header value utf8 error: {err}"
                        ))
                    })?;
                    out.insert(name.to_string(), value.to_string());
                }
                idx += len;
            }
            8 => {
                ensure_len(idx, 8, "timestamp")?;
                idx += 8;
            }
            9 => {
                ensure_len(idx, 16, "uuid")?;
                idx += 16;
            }
            other => {
                return Err(DittoError::InvalidResponse(format!(
                    "eventstream unsupported header type {other}"
                )));
            }
        }
    }
    Ok(out)
}

fn bedrock_event_stream_from_response(
    response: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<String>> {
    stream::unfold(
        (
            response.bytes_stream(),
            EventStreamDecoder::default(),
            VecDeque::<Result<String>>::new(),
        ),
        |(mut bytes_stream, mut decoder, mut buffer)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((item, (bytes_stream, decoder, buffer)));
                }
                let next = bytes_stream.next().await;
                match next {
                    Some(Ok(chunk)) => {
                        decoder.push(&chunk);
                        while let Some(message) = decoder.next_message() {
                            match message {
                                Ok(message) => match parse_bedrock_event(&message) {
                                    Ok(Some(data)) => buffer.push_back(Ok(data)),
                                    Ok(None) => {}
                                    Err(err) => buffer.push_back(Err(err)),
                                },
                                Err(err) => buffer.push_back(Err(err)),
                            }
                        }
                    }
                    Some(Err(err)) => {
                        buffer.push_back(Err(DittoError::Http(err)));
                    }
                    None => return None,
                }
            }
        },
    )
}

fn parse_bedrock_event(message: &EventStreamMessage) -> Result<Option<String>> {
    let message_type = message
        .headers
        .get(":message-type")
        .map(String::as_str)
        .unwrap_or("event");
    if message_type != "event" {
        return Err(DittoError::InvalidResponse(format!(
            "bedrock eventstream message-type={message_type}"
        )));
    }

    let outer: Value = serde_json::from_slice(&message.payload)?;
    let bytes = outer
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| DittoError::InvalidResponse("bedrock event missing bytes".to_string()))?;
    let decoded = BASE64.decode(bytes).map_err(|err| {
        DittoError::InvalidResponse(format!("bedrock base64 decode failed: {err}"))
    })?;
    let json = String::from_utf8(decoded).map_err(|err| {
        DittoError::InvalidResponse(format!("bedrock event bytes not utf8: {err}"))
    })?;
    Ok(Some(json))
}

