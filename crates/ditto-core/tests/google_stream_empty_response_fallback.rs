#![cfg(all(feature = "provider-google", feature = "cap-llm-streaming"))]

use std::collections::BTreeMap;

use ditto_core::config::{Env, ProviderApi, ProviderAuth, ProviderConfig};
use ditto_core::contracts::{FinishReason, Warning};
use ditto_core::contracts::{GenerateRequest, Message, StreamChunk};
use ditto_core::llm_core::model::LanguageModel;
use ditto_core::providers::Google;
use futures_util::StreamExt;
use httpmock::{Method::POST, MockServer};

#[tokio::test]
async fn google_stream_empty_response_error_falls_back_to_generate() -> ditto_core::error::Result<()>
{
    if ditto_core::utils::test_support::should_skip_httpmock() {
        return Ok(());
    }

    let server = MockServer::start_async().await;
    let stream_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1beta/models/gemini-3.1-pro-preview:streamGenerateContent")
                .query_param("alt", "sse")
                .header("authorization", "Bearer yunwu-test-key")
                .header("accept", "text/event-stream");
            then.status(429)
                .header("content-type", "application/json")
                .body(
                    r#"{"error":{"message":"received empty response from Gemini: no meaningful content in candidates","code":"channel:empty_response"}}"#,
                );
        })
        .await;

    let generate_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1beta/models/gemini-3.1-pro-preview:generateContent")
                .header("authorization", "Bearer yunwu-test-key");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"candidates":[{"content":{"parts":[{"text":"OK"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":6,"candidatesTokenCount":1,"totalTokenCount":7}}"#,
                );
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
    let mut saw_fallback_warning = false;
    let mut finish_reason = None;
    while let Some(item) = stream.next().await {
        match item? {
            StreamChunk::Warnings { warnings } => {
                saw_fallback_warning = warnings.iter().any(|warning| {
                    matches!(
                        warning,
                        Warning::Compatibility { feature, .. }
                            if feature == "stream.empty_response_error"
                    )
                });
            }
            StreamChunk::TextDelta { text } => {
                if text == "OK" {
                    saw_ok = true;
                }
            }
            StreamChunk::FinishReason(reason) => finish_reason = Some(reason),
            _ => {}
        }
    }

    stream_mock.assert_async().await;
    generate_mock.assert_async().await;
    assert!(saw_ok, "expected fallback generate text");
    assert!(saw_fallback_warning, "expected compatibility warning");
    assert_eq!(finish_reason, Some(FinishReason::Stop));
    Ok(())
}
