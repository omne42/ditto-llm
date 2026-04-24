#![cfg_attr(not(feature = "cap-llm-streaming"), allow(dead_code))]

use std::collections::VecDeque;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;

use crate::contracts::{StreamChunk, Warning};
use crate::error::{DittoError, Result};

#[allow(dead_code)]
fn map_http_kit_sse_error(error: http_kit::Error) -> DittoError {
    let message = error.message();

    if let Some(limit) = message
        .strip_suffix(" must be greater than zero")
        .filter(|limit| !limit.is_empty())
    {
        return crate::invalid_response!(
            "error_detail.sse.limit_must_be_positive",
            "limit" => limit
        );
    }

    if let Some(max_line_bytes) = message.strip_prefix("sse line exceeds max_line_bytes ") {
        return crate::invalid_response!(
            "error_detail.sse.line_too_large",
            "max_line_bytes" => max_line_bytes
        );
    }

    if let Some(max_event_bytes) = message.strip_prefix("sse event exceeds max_event_bytes ") {
        return crate::invalid_response!(
            "error_detail.sse.event_too_large",
            "max_event_bytes" => max_event_bytes
        );
    }

    if let Some(read_error) = message.strip_prefix("read sse line failed: ") {
        return crate::invalid_response!(
            "error_detail.sse.read_line_failed",
            "error" => read_error
        );
    }

    if let Some(decode_error) = message.strip_prefix("invalid sse utf-8: ") {
        return crate::invalid_response!(
            "error_detail.sse.invalid_utf8",
            "error" => decode_error
        );
    }

    DittoError::invalid_response_text(message)
}

pub(crate) fn init_data_stream(
    data_stream: BoxStream<'static, Result<String>>,
    warnings: Vec<Warning>,
) -> (
    BoxStream<'static, Result<String>>,
    VecDeque<Result<StreamChunk>>,
) {
    let mut buffer = VecDeque::<Result<StreamChunk>>::new();
    if !warnings.is_empty() {
        buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
    }
    (data_stream, buffer)
}

#[cfg_attr(not(feature = "cap-llm-streaming"), allow(dead_code))]
pub(crate) fn init_sse_stream(
    response: reqwest::Response,
    warnings: Vec<Warning>,
) -> (
    BoxStream<'static, Result<String>>,
    VecDeque<Result<StreamChunk>>,
) {
    let data_stream = Box::pin(
        http_kit::sse_data_stream_from_response(response)
            .map(|item| item.map_err(map_http_kit_sse_error)),
    );
    init_data_stream(data_stream, warnings)
}
