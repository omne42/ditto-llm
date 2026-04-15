use futures_util::StreamExt;
use futures_util::stream::BoxStream;
pub use http_kit::SseLimits;
use tokio::io::AsyncBufRead;

use crate::error::{DittoError, Result};

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

pub fn sse_data_stream_from_reader_with_limits<R>(
    reader: R,
    limits: SseLimits,
) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    Box::pin(
        http_kit::sse_data_stream_from_reader_with_limits(reader, limits)
            .map(|item| item.map_err(map_http_kit_sse_error)),
    )
}

pub fn sse_data_stream_from_reader<R>(reader: R) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    Box::pin(
        http_kit::sse_data_stream_from_reader(reader)
            .map(|item| item.map_err(map_http_kit_sse_error)),
    )
}

pub fn sse_data_stream_from_response(
    response: reqwest::Response,
) -> BoxStream<'static, Result<String>> {
    Box::pin(
        http_kit::sse_data_stream_from_response(response)
            .map(|item| item.map_err(map_http_kit_sse_error)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::{StreamExt, stream};

    fn expect_invalid_response_catalog<'a>(
        err: &'a DittoError,
        expected_code: &str,
    ) -> structured_text_kit::CatalogTextRef<'a> {
        let DittoError::InvalidResponse(message) = err else {
            panic!("expected invalid response, got {err}");
        };
        let text = message
            .as_catalog()
            .expect("expected catalog-backed invalid response");
        assert_eq!(text.code(), expected_code);
        text
    }

    #[tokio::test]
    async fn parses_sse_data_lines() -> Result<()> {
        let sse = concat!(
            "event: message\n",
            "data: {\"hello\":1}\n\n",
            "data: line1\n",
            "data: line2\n\n",
        );

        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = tokio_util::io::StreamReader::new(stream);
        let mut out = Vec::new();
        let mut data_stream = sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        while let Some(item) = data_stream.next().await {
            out.push(item?);
        }

        assert_eq!(out, vec!["{\"hello\":1}", "line1\nline2"]);
        Ok(())
    }

    #[tokio::test]
    async fn preserves_empty_data_events_and_done_literal() -> Result<()> {
        let sse = concat!("data:\n\n", "data: [DONE]\n\n");
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = tokio_util::io::StreamReader::new(stream);
        let mut out = Vec::new();
        let mut data_stream = sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        while let Some(item) = data_stream.next().await {
            out.push(item?);
        }

        assert_eq!(out, vec!["", "[DONE]"]);
        Ok(())
    }

    #[tokio::test]
    async fn preserves_single_optional_space_after_data_colon() -> Result<()> {
        let sse = concat!("data:  indented\n", "data:\tkeeps-tab\n", "data\n\n");
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        let item = data_stream.next().await.unwrap()?;
        assert_eq!(item, " indented\n\tkeeps-tab\n");
        assert!(data_stream.next().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn parses_events_across_multiple_stream_chunks() -> Result<()> {
        let stream = stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"data: hel")),
            Ok(Bytes::from_static(b"lo\n")),
            Ok(Bytes::from_static(b"data: wor")),
            Ok(Bytes::from_static(b"ld\n\n")),
        ]);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        let item = data_stream.next().await.unwrap()?;
        assert_eq!(item, "hello\nworld");
        assert!(data_stream.next().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn rejects_sse_lines_over_max_line_bytes() -> Result<()> {
        let sse = format!("data: {}\n\n", "x".repeat(1024));
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = tokio_util::io::StreamReader::new(stream);
        let mut data_stream = sse_data_stream_from_reader_with_limits(
            tokio::io::BufReader::new(reader),
            SseLimits {
                max_line_bytes: 64,
                max_event_bytes: 4096,
            },
        );
        let err = data_stream.next().await.unwrap().unwrap_err();
        let text = expect_invalid_response_catalog(&err, "error_detail.sse.line_too_large");
        assert_eq!(text.text_arg("max_line_bytes"), Some("64"));
        Ok(())
    }

    #[tokio::test]
    async fn rejects_sse_events_over_max_event_bytes() -> Result<()> {
        let sse = format!(
            "data: {}\n\ndata: {}\n\n",
            "a".repeat(1024),
            "b".repeat(1024)
        );
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader_with_limits(
            tokio::io::BufReader::new(reader),
            SseLimits {
                max_line_bytes: 4096,
                max_event_bytes: 128,
            },
        );

        let err = data_stream.next().await.unwrap().unwrap_err();
        let text = expect_invalid_response_catalog(&err, "error_detail.sse.event_too_large");
        assert_eq!(text.text_arg("max_event_bytes"), Some("128"));
        Ok(())
    }

    #[tokio::test]
    async fn accepts_sse_event_exactly_at_max_event_bytes() -> Result<()> {
        let sse = "data: abcde\n\n";
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader_with_limits(
            tokio::io::BufReader::new(reader),
            SseLimits {
                max_line_bytes: 4096,
                max_event_bytes: 5,
            },
        );

        let first = data_stream.next().await.unwrap()?;
        assert_eq!(first, "abcde");
        assert!(data_stream.next().await.is_none());
        Ok(())
    }
}
