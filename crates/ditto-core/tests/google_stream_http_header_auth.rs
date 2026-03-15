#![cfg(all(feature = "provider-google", feature = "cap-llm-streaming"))]

use std::collections::BTreeMap;

use ditto_core::config::{Env, ProviderApi, ProviderAuth, ProviderConfig};
use ditto_core::contracts::FinishReason;
use ditto_core::contracts::{GenerateRequest, Message, StreamChunk};
use ditto_core::llm_core::model::LanguageModel;
use ditto_core::providers::Google;
use futures_util::StreamExt;
use httpmock::{Method::POST, MockServer};

#[tokio::test]
async fn google_stream_supports_authorization_bearer_header_auth() -> ditto_core::error::Result<()>
{
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let server = MockServer::start_async().await;
    let sse = concat!(
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"OK\"}]},\"finishReason\":\"\",\"index\":0}],\"modelVersion\":\"gemini-3.1-pro-preview\"}\n\n",
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"thoughtSignature\":\"sig-1\"}]},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":6,\"candidatesTokenCount\":1,\"totalTokenCount\":119,\"thoughtsTokenCount\":112}}\n\n",
    );

    let stream_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1beta/models/gemini-3.1-pro-preview:streamGenerateContent")
                .query_param("alt", "sse")
                .header("authorization", "Bearer yunwu-test-key")
                .header("accept", "text/event-stream");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse);
        })
        .await;

    let client = Google::from_config(
        &ProviderConfig {
            base_url: Some(server.url("/v1beta")),
            default_model: Some("gemini-3.1-pro-preview".to_string()),
            auth: Some(ProviderAuth::HttpHeaderEnv {
                header: "Authorization".to_string(),
                keys: vec!["YUNWU_API_KEY".to_string()],
                prefix: Some("Bearer ".to_string()),
            }),
            upstream_api: Some(ProviderApi::GeminiGenerateContent),
            ..ProviderConfig::default()
        },
        &Env {
            dotenv: BTreeMap::from([("YUNWU_API_KEY".to_string(), "yunwu-test-key".to_string())]),
        },
    )
    .await?;

    let mut stream = client
        .stream(GenerateRequest::from(vec![Message::user(
            "Reply with OK only.",
        )]))
        .await?;

    let mut saw_ok = false;
    let mut finish_reason = None;
    let mut total_tokens = None;
    while let Some(item) = stream.next().await {
        match item? {
            StreamChunk::TextDelta { text } => {
                if text == "OK" {
                    saw_ok = true;
                }
            }
            StreamChunk::FinishReason(reason) => finish_reason = Some(reason),
            StreamChunk::Usage(usage) => total_tokens = usage.total_tokens,
            _ => {}
        }
    }

    stream_mock.assert_async().await;
    assert!(saw_ok, "expected streamed text delta");
    assert_eq!(finish_reason, Some(FinishReason::Stop));
    assert_eq!(total_tokens, Some(119));
    Ok(())
}
