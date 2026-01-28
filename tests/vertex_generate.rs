#![cfg(feature = "vertex")]

use ditto_llm::auth::OAuthClientCredentials;
use ditto_llm::{GenerateRequest, LanguageModel, Message, Tool, ToolChoice, Vertex};
use httpmock::{Method::POST, MockServer};
use serde_json::json;

fn approx_eq(actual: Option<f64>, expected: f64) -> bool {
    const EPSILON: f64 = 1e-6;
    actual.is_some_and(|value| (value - expected).abs() <= EPSILON)
}

#[tokio::test]
async fn vertex_generate_maps_genai_request() -> ditto_llm::Result<()> {
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
                .body_includes("client_secret=secret-a")
                .body_includes("scope=scope-a");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"access_token":"token-abc","token_type":"Bearer"}"#);
        })
        .await;

    let generate_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/models/gemini-pro:generateContent")
                .header("authorization", "Bearer token-abc")
                .is_true(|req: &httpmock::prelude::HttpMockRequest| {
                    let Ok(body) = serde_json::from_slice::<serde_json::Value>(req.body_ref())
                    else {
                        return false;
                    };

                    let Some(contents) = body.get("contents").and_then(|v| v.as_array()) else {
                        return false;
                    };
                    if !contents
                        .iter()
                        .any(|item| item.get("role").and_then(|v| v.as_str()) == Some("user"))
                    {
                        return false;
                    }

                    let Some(system_parts) = body
                        .get("systemInstruction")
                        .and_then(|v| v.get("parts"))
                        .and_then(|v| v.as_array())
                    else {
                        return false;
                    };
                    if !system_parts
                        .iter()
                        .any(|part| part.get("text").and_then(|v| v.as_str()) == Some("sys"))
                    {
                        return false;
                    }

                    let Some(config) = body.get("generationConfig") else {
                        return false;
                    };
                    if config.get("maxOutputTokens").and_then(|v| v.as_u64()) != Some(12) {
                        return false;
                    }
                    if !approx_eq(config.get("temperature").and_then(|v| v.as_f64()), 0.2) {
                        return false;
                    }
                    if !approx_eq(config.get("topP").and_then(|v| v.as_f64()), 0.9) {
                        return false;
                    }
                    let Some(stops) = config.get("stopSequences").and_then(|v| v.as_array()) else {
                        return false;
                    };
                    if !stops.iter().any(|v| v.as_str() == Some("done")) {
                        return false;
                    }

                    let Some(tools) = body.get("tools").and_then(|v| v.as_array()) else {
                        return false;
                    };
                    if !tools.iter().any(|tool| {
                        tool.get("functionDeclarations")
                            .and_then(|v| v.as_array())
                            .is_some_and(|decls| {
                                decls.iter().any(|decl| {
                                    decl.get("name").and_then(|v| v.as_str()) == Some("add")
                                        && decl.get("description").and_then(|v| v.as_str())
                                            == Some("add")
                                })
                            })
                    }) {
                        return false;
                    }

                    body.pointer("/toolConfig/functionCallingConfig/mode")
                        .and_then(|v| v.as_str())
                        == Some("ANY")
                        && body
                            .pointer("/toolConfig/functionCallingConfig/allowedFunctionNames")
                            .and_then(|v| v.as_array())
                            .is_some_and(|names| {
                                names.iter().any(|name| name.as_str() == Some("add"))
                            })
                });
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    json!({
                        "candidates": [{
                            "content": { "parts": [{ "text": "ok" }] },
                            "finishReason": "STOP"
                        }],
                        "usageMetadata": {
                            "promptTokenCount": 1,
                            "candidatesTokenCount": 2,
                            "totalTokenCount": 3
                        }
                    })
                    .to_string(),
                );
        })
        .await;

    let oauth = OAuthClientCredentials::new(server.url("/token"), "client-a", "secret-a")?
        .with_scope("scope-a");
    let client = Vertex::new(oauth, server.url("/v1"), "gemini-pro")?;

    let tool = Tool {
        name: "add".to_string(),
        description: Some("add".to_string()),
        parameters: json!({
            "type": "object",
            "properties": { "a": { "type": "integer" } }
        }),
        strict: None,
    };

    let mut request = GenerateRequest::from(vec![Message::system("sys"), Message::user("hi")]);
    request.max_tokens = Some(12);
    request.temperature = Some(0.2);
    request.top_p = Some(0.9);
    request.stop_sequences = Some(vec!["done".to_string()]);
    request.tools = Some(vec![tool]);
    request.tool_choice = Some(ToolChoice::Tool {
        name: "add".to_string(),
    });

    let response = client.generate(request).await?;

    token_mock.assert_async().await;
    generate_mock.assert_async().await;
    assert_eq!(response.text(), "ok".to_string());
    Ok(())
}
