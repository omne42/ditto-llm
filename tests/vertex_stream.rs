#![cfg(all(feature = "vertex", feature = "streaming"))]

use ditto_llm::auth::OAuthClientCredentials;
use ditto_llm::{FinishReason, GenerateRequest, LanguageModel, Message, StreamChunk, Vertex};
use futures_util::StreamExt;
use httpmock::{Method::POST, MockServer};

#[tokio::test]
async fn vertex_stream_parses_sse_chunks() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let server = MockServer::start_async().await;
    let token_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/token")
                .body_includes("grant_type=client_credentials")
                .body_includes("client_id=client-a")
                .body_includes("client_secret=secret-a");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"access_token":"token-abc","token_type":"Bearer"}"#);
        })
        .await;

    let sse = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello world\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"add\",\"args\":{\"a\":1}}}]}}]}\n\n",
        "data: {\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":2,\"totalTokenCount\":3},\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n",
        "data: [DONE]\n\n",
    );

    let stream_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/models/gemini-pro:streamGenerateContent")
                .query_param("alt", "sse")
                .header("authorization", "Bearer token-abc")
                .header("accept", "text/event-stream");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse);
        })
        .await;

    let oauth = OAuthClientCredentials::new(server.url("/token"), "client-a", "secret-a")?;
    let client = Vertex::new(oauth, server.url("/v1"), "gemini-pro")?;
    let request = GenerateRequest::from(vec![Message::user("hi")]);

    let mut stream = client.stream(request).await?;
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item?);
    }

    token_mock.assert_async().await;
    stream_mock.assert_async().await;

    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::TextDelta { text } if text == "Hello"))
    );
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::TextDelta { text } if text == " world"))
    );
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ToolCallStart { name, .. } if name == "add"))
    );
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ToolCallDelta { arguments_delta, .. } if arguments_delta.contains("\"a\":1")))
    );

    let finish = chunks.iter().find_map(|chunk| match chunk {
        StreamChunk::FinishReason(reason) => Some(*reason),
        _ => None,
    });
    assert_eq!(finish, Some(FinishReason::ToolCalls));

    let usage = chunks.iter().find_map(|chunk| match chunk {
        StreamChunk::Usage(usage) => Some(usage),
        _ => None,
    });
    assert_eq!(usage.and_then(|u| u.total_tokens), Some(3));

    Ok(())
}
