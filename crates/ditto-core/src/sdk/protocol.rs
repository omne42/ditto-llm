use serde::{Deserialize, Serialize};

use crate::contracts::StreamChunk;
use crate::error::Result;

pub const STREAM_PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StreamEventV1 {
    Chunk(StreamChunk),
    Done,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEnvelopeV1 {
    pub v: u8,
    #[serde(flatten)]
    pub event: StreamEventV1,
}

#[derive(Serialize)]
struct StreamEnvelopeV1Ref<'a> {
    v: u8,
    #[serde(flatten)]
    event: &'a StreamEventV1,
}

impl StreamEnvelopeV1 {
    pub fn new(event: StreamEventV1) -> Self {
        Self {
            v: STREAM_PROTOCOL_VERSION,
            event,
        }
    }
}

pub fn encode_v1(event: &StreamEventV1) -> Result<String> {
    let envelope = StreamEnvelopeV1Ref {
        v: STREAM_PROTOCOL_VERSION,
        event,
    };
    Ok(serde_json::to_string(&envelope)?)
}

pub fn encode_v1_bytes(event: &StreamEventV1) -> Result<Vec<u8>> {
    let envelope = StreamEnvelopeV1Ref {
        v: STREAM_PROTOCOL_VERSION,
        event,
    };
    Ok(serde_json::to_vec(&envelope)?)
}

pub fn encode_line_v1(event: &StreamEventV1) -> Result<String> {
    let mut line = encode_v1(event)?;
    line.push('\n');
    Ok(line)
}

pub fn decode_v1(input: &str) -> Result<StreamEventV1> {
    let envelope: StreamEnvelopeV1 = serde_json::from_str(input.trim())?;
    if envelope.v != STREAM_PROTOCOL_VERSION {
        return Err(crate::invalid_response!(
            "error_detail.stream_protocol.unsupported_version",
            "version" => envelope.v.to_string()
        ));
    }
    Ok(envelope.event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::StreamChunk;

    #[test]
    fn protocol_roundtrip_v1() {
        let event = StreamEventV1::Chunk(StreamChunk::TextDelta {
            text: "hello".to_string(),
        });
        let encoded = encode_v1(&event).expect("encode");
        let decoded = decode_v1(&encoded).expect("decode");
        assert_eq!(decoded, event);
    }
}
