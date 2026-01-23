use futures_util::TryStreamExt;
use futures_util::stream::{self, BoxStream};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio_util::io::StreamReader;

use crate::Result;

async fn read_next_sse_data<R>(
    lines: &mut tokio::io::Lines<R>,
    buffer: &mut String,
) -> Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    buffer.clear();

    loop {
        let Some(line) = lines.next_line().await? else {
            if buffer.is_empty() {
                return Ok(None);
            }
            let data = std::mem::take(buffer);
            return Ok(Some(data));
        };

        let line = line.trim_end_matches('\r');
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
            if !buffer.is_empty() {
                buffer.push('\n');
            }
            buffer.push_str(rest);
        }
    }
}

pub fn sse_data_stream_from_lines<R>(
    lines: tokio::io::Lines<R>,
) -> BoxStream<'static, Result<String>>
where
    R: AsyncBufRead + Unpin + Send + 'static,
{
    Box::pin(stream::try_unfold(
        (lines, String::new()),
        |(mut lines, mut buffer)| async move {
            match read_next_sse_data(&mut lines, &mut buffer).await? {
                Some(data) => Ok(Some((data, (lines, buffer)))),
                None => Ok(None),
            }
        },
    ))
}

pub fn sse_data_stream_from_response(
    response: reqwest::Response,
) -> BoxStream<'static, Result<String>> {
    let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
    let reader = StreamReader::new(byte_stream);
    let lines = tokio::io::BufReader::new(reader).lines();
    sse_data_stream_from_lines(lines)
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
        let lines = tokio::io::BufReader::new(reader).lines();
        let mut out = Vec::new();
        let mut data_stream = sse_data_stream_from_lines(lines);
        while let Some(item) = data_stream.next().await {
            out.push(item?);
        }

        assert_eq!(out, vec!["{\"hello\":1}", "line1\nline2"]);
        Ok(())
    }
}
