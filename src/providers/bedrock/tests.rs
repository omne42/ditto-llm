#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    #[cfg(feature = "streaming")]
    fn build_event_stream_message(payload: &Value) -> Vec<u8> {
        let payload_bytes = serde_json::to_vec(payload).expect("payload json");

        let mut headers = Vec::<u8>::new();
        let name = ":message-type".as_bytes();
        headers.push(name.len() as u8);
        headers.extend_from_slice(name);
        headers.push(7u8); // string
        let value = "event".as_bytes();
        headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
        headers.extend_from_slice(value);

        let headers_len = headers.len();
        let total_len = 12 + headers_len + payload_bytes.len() + 4;
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(&(total_len as u32).to_be_bytes());
        out.extend_from_slice(&(headers_len as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // prelude crc (ignored)
        out.extend_from_slice(&headers);
        out.extend_from_slice(&payload_bytes);
        out.extend_from_slice(&0u32.to_be_bytes()); // message crc (ignored)
        out
    }

    #[cfg(feature = "streaming")]
    fn bedrock_event(inner: Value) -> Value {
        let bytes = BASE64.encode(serde_json::to_vec(&inner).expect("inner json"));
        json!({
            "bytes": bytes
        })
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn eventstream_decoder_invalid_total_len_is_consumed() {
        let mut decoder = EventStreamDecoder::default();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(15u32).to_be_bytes());
        frame.extend_from_slice(&0u32.to_be_bytes());
        frame.extend_from_slice(&0u32.to_be_bytes());
        decoder.push(&frame).expect("push frame");

        let first = decoder.next_message().expect("invalid frame should produce error");
        assert!(first.is_err());
        assert!(
            decoder.next_message().is_none(),
            "decoder must advance after invalid frame"
        );
    }

    #[tokio::test]
    async fn bedrock_generate_maps_anthropic_body() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let signer = SigV4Signer::new("AKID", "SECRET", None, "us-east-1", "bedrock")?;
        let client = Bedrock::new(signer, server.url(""), "claude-test")?;

        let expected_body = json!({
            "anthropic_version": DEFAULT_VERSION,
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "hi"}]
            }],
            "max_tokens": 1024
        });

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/model/claude-test/invoke")
                    .json_body_includes(expected_body.to_string());
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        json!({
                            "content": [{ "type": "text", "text": "ok" }],
                            "stop_reason": "stop",
                            "usage": { "input_tokens": 1, "output_tokens": 2 }
                        })
                        .to_string(),
                    );
            })
            .await;

        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let response = client.generate(request).await?;
        mock.assert_async().await;
        assert_eq!(response.text(), "ok");
        Ok(())
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn bedrock_stream_parses_eventstream() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let signer = SigV4Signer::new("AKID", "SECRET", None, "us-east-1", "bedrock")?;
        let client = Bedrock::new(signer, server.url(""), "claude-test")?;

        let events = vec![
            bedrock_event(json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            })),
            bedrock_event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Hello" }
            })),
            bedrock_event(json!({
                "type": "message_delta",
                "usage": { "input_tokens": 1, "output_tokens": 2 },
                "delta": { "stop_reason": "stop" }
            })),
            bedrock_event(json!({
                "type": "message_stop"
            })),
        ];
        let mut stream_body = Vec::<u8>::new();
        for event in events {
            stream_body.extend(build_event_stream_message(&event));
        }

        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/model/claude-test/invoke-with-response-stream");
                then.status(200)
                    .header("content-type", "application/vnd.amazon.eventstream")
                    .body(stream_body);
            })
            .await;

        let request = GenerateRequest::from(vec![Message::user("hi")]);
        let mut stream = client.stream(request).await?;
        let mut chunks = Vec::new();
        while let Some(item) = stream.next().await {
            chunks.push(item?);
        }

        mock.assert_async().await;
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::TextDelta { text } if text == "Hello"))
        );
        assert!(
            chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::FinishReason(FinishReason::Stop)))
        );
        Ok(())
    }
}
