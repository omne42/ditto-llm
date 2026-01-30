//! HTTP adapters for Ditto streaming protocol v1.

use std::collections::VecDeque;
use std::io::{self, Result as IoResult};

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};

use crate::{DittoError, StreamResult};

use super::protocol::{StreamEventV1, encode_line_v1, encode_v1};

#[derive(Clone, Copy)]
enum HttpStreamFormat {
    Ndjson,
    Sse,
}

impl HttpStreamFormat {
    fn encode(self, event: &StreamEventV1) -> IoResult<Bytes> {
        match self {
            Self::Ndjson => Ok(Bytes::from(encode_line_v1(event).map_err(to_io_error)?)),
            Self::Sse => {
                let json = encode_v1(event).map_err(to_io_error)?;
                Ok(Bytes::from(format!("data: {json}\n\n")))
            }
        }
    }
}

fn to_io_error(err: DittoError) -> io::Error {
    io::Error::other(err.to_string())
}

fn stream_v1_http(
    stream: StreamResult,
    format: HttpStreamFormat,
) -> BoxStream<'static, IoResult<Bytes>> {
    stream::unfold(
        (stream, VecDeque::<IoResult<Bytes>>::new(), false),
        move |(mut inner, mut buffer, mut done)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    return Some((item, (inner, buffer, done)));
                }
                if done {
                    return None;
                }

                match inner.next().await {
                    Some(Ok(chunk)) => {
                        let event = StreamEventV1::Chunk(chunk);
                        let item = format.encode(&event);
                        if item.is_err() {
                            done = true;
                        }
                        buffer.push_back(item);
                    }
                    Some(Err(err)) => {
                        let item = format.encode(&StreamEventV1::Error {
                            message: err.to_string(),
                        });
                        if item.is_err() {
                            done = true;
                            buffer.push_back(item);
                            continue;
                        }
                        buffer.push_back(item);

                        let item = format.encode(&StreamEventV1::Done);
                        buffer.push_back(item);

                        done = true;
                    }
                    None => {
                        let item = format.encode(&StreamEventV1::Done);
                        buffer.push_back(item);
                        done = true;
                    }
                }
            }
        },
    )
    .boxed()
}

/// Convert a `StreamResult` into Ditto stream protocol v1 NDJSON (`<json>\n`).
///
/// Guarantees `StreamEventV1::Done` at end. If the upstream stream yields `Err(DittoError)`,
/// emits `StreamEventV1::Error { message }` followed by `Done`, then terminates.
pub fn stream_v1_ndjson(stream: StreamResult) -> BoxStream<'static, IoResult<Bytes>> {
    stream_v1_http(stream, HttpStreamFormat::Ndjson)
}

/// Convert a `StreamResult` into Ditto stream protocol v1 SSE (`data: <json>\n\n`).
///
/// Guarantees `StreamEventV1::Done` at end. If the upstream stream yields `Err(DittoError)`,
/// emits `StreamEventV1::Error { message }` followed by `Done`, then terminates.
pub fn stream_v1_sse(stream: StreamResult) -> BoxStream<'static, IoResult<Bytes>> {
    stream_v1_http(stream, HttpStreamFormat::Sse)
}

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
