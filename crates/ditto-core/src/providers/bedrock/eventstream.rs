#[derive(Debug)]
struct EventStreamMessage {
    headers: HashMap<String, String>,
    payload: bytes::Bytes,
}

#[derive(Debug, Default)]
struct EventStreamDecoder {
    buffer: bytes::BytesMut,
}

const MAX_EVENTSTREAM_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTSTREAM_BUFFER_BYTES: usize = 8 * 1024 * 1024;

impl EventStreamDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<()> {
        if self
            .buffer
            .len()
            .saturating_add(chunk.len())
            .gt(&MAX_EVENTSTREAM_BUFFER_BYTES)
        {
            self.buffer.clear();
            return Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_buffer_exceeded",
                "max_bytes" => MAX_EVENTSTREAM_BUFFER_BYTES.to_string()
            ));
        }
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    fn next_message(&mut self) -> Option<Result<EventStreamMessage>> {
        if self.buffer.len() < 12 {
            return None;
        }
        let total_len = u32::from_be_bytes(self.buffer[0..4].try_into().ok()?) as usize;
        if total_len < 16 {
            self.buffer.clear();
            return Some(Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_total_len_too_small"
            )));
        }
        if total_len > MAX_EVENTSTREAM_MESSAGE_BYTES {
            self.buffer.clear();
            return Some(Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_total_len_too_large",
                "total_len" => total_len.to_string(),
                "max_bytes" => MAX_EVENTSTREAM_MESSAGE_BYTES.to_string()
            )));
        }
        if self.buffer.len() < total_len {
            return None;
        }
        let headers_len = u32::from_be_bytes(self.buffer[4..8].try_into().ok()?) as usize;
        let headers_start = 12usize;
        let headers_end = headers_start.saturating_add(headers_len);
        if headers_end > total_len {
            self.buffer.clear();
            return Some(Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_invalid_headers_length"
            )));
        }
        let payload_end = total_len.saturating_sub(4);
        if headers_end > payload_end {
            self.buffer.clear();
            return Some(Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_invalid_payload_length"
            )));
        }

        // split_to keeps remaining bytes in-place without front-drain memmove.
        let frame = self.buffer.split_to(total_len).freeze();
        let headers_result = parse_eventstream_headers(&frame[headers_start..headers_end]);
        let payload = frame.slice(headers_end..payload_end);
        let headers = match headers_result {
            Ok(headers) => headers,
            Err(err) => return Some(Err(err)),
        };
        Some(Ok(EventStreamMessage { headers, payload }))
    }
}

fn parse_eventstream_headers(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::<String, String>::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let name_len = *bytes
            .get(idx)
            .ok_or_else(|| {
                crate::invalid_response!(
                    "error_detail.bedrock.eventstream_header_missing_name_length"
                )
            })? as usize;
        idx += 1;
        if idx + name_len > bytes.len() {
            return Err(crate::invalid_response!(
                "error_detail.bedrock.eventstream_header_name_truncated"
            ));
        }
        let name = std::str::from_utf8(&bytes[idx..idx + name_len]).map_err(|err| {
            crate::invalid_response!(
                "error_detail.bedrock.eventstream_bad_header_name",
                "error" => err.to_string()
            )
        })?;
        idx += name_len;
        let value_type = *bytes
            .get(idx)
            .ok_or_else(|| {
                crate::invalid_response!("error_detail.bedrock.eventstream_header_missing_type")
            })?;
        idx += 1;
        let ensure_len = |idx: usize, needed: usize, label: &str| -> Result<()> {
            if idx + needed > bytes.len() {
                return Err(crate::invalid_response!(
                    "error_detail.bedrock.eventstream_header_value_truncated",
                    "label" => label
                ));
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
                        crate::invalid_response!(
                            "error_detail.bedrock.eventstream_header_value_utf8_error",
                            "error" => err.to_string()
                        )
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
                return Err(crate::invalid_response!(
                    "error_detail.bedrock.eventstream_unsupported_header_type",
                    "header_type" => other.to_string()
                ));
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
                        if let Err(err) = decoder.push(&chunk) {
                            buffer.push_back(Err(err));
                            continue;
                        }
                        while let Some(message) = decoder.next_message() {
                            match message {
                                Ok(message) => match parse_bedrock_event(&message) {
                                    Ok(Some(data)) => buffer.push_back(Ok(data)),
                                    Ok(None) => {}
                                    Err(err) => buffer.push_back(Err(err)),
                                },
                                Err(err) => {
                                    buffer.push_back(Err(err));
                                    break;
                                }
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
        return Err(crate::invalid_response!(
            "error_detail.bedrock.eventstream_message_type_invalid",
            "message_type" => message_type
        ));
    }

    let outer: Value = serde_json::from_slice(&message.payload)?;
    let bytes = outer
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::invalid_response!("error_detail.bedrock.event_missing_bytes"))?;
    let decoded = BASE64.decode(bytes).map_err(|err| {
        crate::invalid_response!(
            "error_detail.bedrock.event_base64_decode_failed",
            "error" => err.to_string()
        )
    })?;
    let json = String::from_utf8(decoded).map_err(|err| {
        crate::invalid_response!(
            "error_detail.bedrock.event_bytes_not_utf8",
            "error" => err.to_string()
        )
    })?;
    Ok(Some(json))
}
