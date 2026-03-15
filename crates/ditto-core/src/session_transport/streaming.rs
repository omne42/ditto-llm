#![cfg_attr(not(feature = "cap-llm-streaming"), allow(dead_code))]

use std::collections::VecDeque;

use futures_util::stream::BoxStream;

use crate::contracts::{StreamChunk, Warning};
use crate::error::Result;

#[cfg_attr(not(feature = "cap-llm-streaming"), allow(dead_code))]
pub(crate) fn init_sse_stream(
    response: reqwest::Response,
    warnings: Vec<Warning>,
) -> (
    BoxStream<'static, Result<String>>,
    VecDeque<Result<StreamChunk>>,
) {
    let data_stream = super::sse::sse_data_stream_from_response(response);
    let mut buffer = VecDeque::<Result<StreamChunk>>::new();
    if !warnings.is_empty() {
        buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
    }
    (data_stream, buffer)
}
