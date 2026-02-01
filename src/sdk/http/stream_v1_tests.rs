#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use futures_util::stream;

    use crate::DittoError;
    use crate::StreamResult;
    use crate::sdk::protocol::{StreamEventV1, decode_v1};
    use crate::types::StreamChunk;

    async fn collect_ndjson_events(stream: StreamResult) -> Vec<StreamEventV1> {
        let mut out = super::stream_v1_ndjson(stream);
        let mut events = Vec::<StreamEventV1>::new();
        while let Some(item) = out.next().await {
            let bytes = item.expect("stream item");
            let text = String::from_utf8(bytes.to_vec()).expect("utf8");
            events.push(decode_v1(&text).expect("decode"));
        }
        events
    }

    async fn collect_sse_events(stream: StreamResult) -> Vec<StreamEventV1> {
        let mut out = super::stream_v1_sse(stream);
        let mut events = Vec::<StreamEventV1>::new();
        while let Some(item) = out.next().await {
            let bytes = item.expect("stream item");
            let text = String::from_utf8(bytes.to_vec()).expect("utf8");
            let json = text
                .strip_prefix("data: ")
                .and_then(|s| s.strip_suffix("\n\n"))
                .expect("sse frame");
            events.push(decode_v1(json).expect("decode"));
        }
        events
    }

    #[tokio::test]
    async fn http_ndjson_appends_done() {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: "world".to_string(),
            }),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let events = collect_ndjson_events(stream).await;
        assert_eq!(
            events,
            vec![
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "hello".to_string()
                }),
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "world".to_string()
                }),
                StreamEventV1::Done,
            ]
        );
    }

    #[tokio::test]
    async fn http_ndjson_error_then_done() {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Err(DittoError::InvalidResponse("boom".to_string())),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let events = collect_ndjson_events(stream).await;
        assert_eq!(
            events,
            vec![
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "hello".to_string()
                }),
                StreamEventV1::Error {
                    message: "invalid response: boom".to_string(),
                },
                StreamEventV1::Done,
            ]
        );
    }

    #[tokio::test]
    async fn http_sse_appends_done() {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Ok(StreamChunk::TextDelta {
                text: "world".to_string(),
            }),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let events = collect_sse_events(stream).await;
        assert_eq!(
            events,
            vec![
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "hello".to_string()
                }),
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "world".to_string()
                }),
                StreamEventV1::Done,
            ]
        );
    }

    #[tokio::test]
    async fn http_sse_error_then_done() {
        let chunks = vec![
            Ok(StreamChunk::TextDelta {
                text: "hello".to_string(),
            }),
            Err(DittoError::InvalidResponse("boom".to_string())),
        ];
        let stream: StreamResult = stream::iter(chunks).boxed();

        let events = collect_sse_events(stream).await;
        assert_eq!(
            events,
            vec![
                StreamEventV1::Chunk(StreamChunk::TextDelta {
                    text: "hello".to_string()
                }),
                StreamEventV1::Error {
                    message: "invalid response: boom".to_string(),
                },
                StreamEventV1::Done,
            ]
        );
    }
}
