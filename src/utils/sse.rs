use futures_util::TryStreamExt;
use futures_util::stream::{self, BoxStream};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio_util::io::StreamReader;

use crate::Result;

#[derive(Clone, Copy, Debug)]
pub struct SseLimits {
    pub max_line_bytes: usize,
    pub max_event_bytes: usize,
}

impl Default for SseLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 256 * 1024,
            max_event_bytes: 4 * 1024 * 1024,
        }
    }
}

async fn read_next_line_bytes_limited<R>(
    reader: &mut R,
    out: &mut Vec<u8>,
    max_bytes: usize,
) -> Result<bool>
where
    R: AsyncBufRead + Unpin,
{
    if max_bytes == 0 {
        return Err(crate::DittoError::InvalidResponse(
            "max_bytes must be > 0".to_string(),
        ));
    }

    out.clear();

    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            return Ok(!out.is_empty());
        }

        let newline_pos = buf.iter().position(|b| *b == b'\n');
        let take_len = newline_pos.map(|pos| pos + 1).unwrap_or(buf.len());

        if out.len().saturating_add(take_len) > max_bytes {
            return Err(crate::DittoError::InvalidResponse(format!(
                "SSE line exceeds max_line_bytes={max_bytes}"
            )));
        }

        out.extend_from_slice(&buf[..take_len]);
        reader.consume(take_len);

        if newline_pos.is_some() {
            return Ok(true);
        }
    }
}

async fn read_next_sse_data_with_limits<R>(
    reader: &mut R,
    line_bytes: &mut Vec<u8>,
    buffer: &mut String,
    limits: SseLimits,
) -> Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    if limits.max_event_bytes == 0 {
        return Err(crate::DittoError::InvalidResponse(
            "max_event_bytes must be > 0".to_string(),
        ));
    }

    buffer.clear();

    loop {
        let has_line = read_next_line_bytes_limited(reader, line_bytes, limits.max_line_bytes)
            .await
            .map_err(|err| {
                crate::DittoError::InvalidResponse(format!("failed to read SSE line: {err}"))
            })?;
        if !has_line {
            if buffer.is_empty() {
                return Ok(None);
            }
            let data = std::mem::take(buffer);
            return Ok(Some(data));
        }

        let line = std::str::from_utf8(line_bytes).map_err(|err| {
            crate::DittoError::InvalidResponse(format!("invalid SSE UTF-8: {err}"))
        })?;
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            if buffer.is_empty() {
                continue;
            }
            if buffer == "[DONE]" {
                return Ok(None);
            }
            let data = std::mem::take(buffer);
            return Ok(Some(data));
        }

        if let Some(rest) = line.strip_prefix("data:") {
            let rest = rest.trim_start();
            let separator_bytes = usize::from(!buffer.is_empty());
            if buffer
                .len()
                .saturating_add(separator_bytes)
                .saturating_add(rest.len())
                > limits.max_event_bytes
            {
                return Err(crate::DittoError::InvalidResponse(format!(
                    "SSE event exceeds max_event_bytes={}",
                    limits.max_event_bytes
                )));
            }
            if separator_bytes == 1 {
                buffer.push('\n');
            }
            buffer.push_str(rest);
        }
    }
}

pub fn sse_data_stream_from_reader_with_limits<R>(
    reader: R,
    limits: SseLimits,
) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    Box::pin(stream::try_unfold(
        (reader, Vec::<u8>::new(), String::new(), limits),
        |(mut reader, mut line_bytes, mut buffer, limits)| async move {
            match read_next_sse_data_with_limits(&mut reader, &mut line_bytes, &mut buffer, limits)
                .await?
            {
                Some(data) => Ok(Some((data, (reader, line_bytes, buffer, limits)))),
                None => Ok(None),
            }
        },
    ))
}

pub fn sse_data_stream_from_reader<R>(reader: R) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    sse_data_stream_from_reader_with_limits(reader, SseLimits::default())
}

#[deprecated(note = "use sse_data_stream_from_reader(_with_limits) for bounded parsing")]
pub fn sse_data_stream_from_lines<R>(
    lines: tokio::io::Lines<R>,
) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    Box::pin(stream::try_unfold(
        (lines, String::new(), SseLimits::default()),
        |(mut lines, mut buffer, limits)| async move {
            buffer.clear();

            loop {
                let Some(line) = lines.next_line().await? else {
                    if buffer.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some((std::mem::take(&mut buffer), (lines, buffer, limits))));
                };

                if line.len() > limits.max_line_bytes {
                    return Err(crate::DittoError::InvalidResponse(format!(
                        "SSE line exceeds max_line_bytes={}",
                        limits.max_line_bytes
                    )));
                }

                let line = line.trim_end_matches('\r');
                if line.is_empty() {
                    if buffer.is_empty() {
                        continue;
                    }
                    if buffer == "[DONE]" {
                        return Ok(None);
                    }
                    return Ok(Some((std::mem::take(&mut buffer), (lines, buffer, limits))));
                }

                if let Some(rest) = line.strip_prefix("data:") {
                    let rest = rest.trim_start();
                    let separator_bytes = usize::from(!buffer.is_empty());
                    if buffer
                        .len()
                        .saturating_add(separator_bytes)
                        .saturating_add(rest.len())
                        > limits.max_event_bytes
                    {
                        return Err(crate::DittoError::InvalidResponse(format!(
                            "SSE event exceeds max_event_bytes={}",
                            limits.max_event_bytes
                        )));
                    }
                    if separator_bytes == 1 {
                        buffer.push('\n');
                    }
                    buffer.push_str(rest);
                }
            }
        },
    ))
}

pub fn sse_data_stream_from_response(
    response: reqwest::Response,
) -> BoxStream<'static, Result<String>> {
    let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
    let reader = StreamReader::new(byte_stream);
    sse_data_stream_from_reader(tokio::io::BufReader::new(reader))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::StreamExt;
    use futures_util::stream;

    #[tokio::test]
    async fn parses_sse_data_lines() -> crate::Result<()> {
        let sse = concat!(
            "event: message\n",
            "data: {\"hello\":1}\n\n",
            "data: line1\n",
            "data: line2\n\n",
            "data: [DONE]\n\n",
        );

        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = StreamReader::new(stream);
        let mut out = Vec::new();
        let mut data_stream = sse_data_stream_from_reader(tokio::io::BufReader::new(reader));
        while let Some(item) = data_stream.next().await {
            out.push(item?);
        }

        assert_eq!(out, vec!["{\"hello\":1}", "line1\nline2"]);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_sse_lines_over_max_line_bytes() -> crate::Result<()> {
        let sse = format!("data: {}\n\n", "x".repeat(1024));
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader_with_limits(
            tokio::io::BufReader::new(reader),
            SseLimits {
                max_line_bytes: 64,
                max_event_bytes: 4096,
            },
        );

        let err = data_stream.next().await.unwrap().unwrap_err();
        assert!(err.to_string().contains("max_line_bytes"));
        Ok(())
    }

    #[tokio::test]
    async fn rejects_sse_events_over_max_event_bytes() -> crate::Result<()> {
        let sse = format!(
            "data: {}\n\ndata: {}\n\n",
            "a".repeat(1024),
            "b".repeat(1024)
        );
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = StreamReader::new(stream);

        let mut data_stream = sse_data_stream_from_reader_with_limits(
            tokio::io::BufReader::new(reader),
            SseLimits {
                max_line_bytes: 4096,
                max_event_bytes: 128,
            },
        );

        let err = data_stream.next().await.unwrap().unwrap_err();
        assert!(err.to_string().contains("max_event_bytes"));
        Ok(())
    }

    #[tokio::test]
    async fn accepts_sse_event_exactly_at_max_event_bytes() -> crate::Result<()> {
        let sse = "data: abcde\n\n";
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse))]);
        let reader = StreamReader::new(stream);

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
