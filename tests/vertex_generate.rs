#![cfg(feature = "vertex")]

use ditto_llm::auth::OAuthClientCredentials;
use ditto_llm::{GenerateRequest, LanguageModel, Message, Tool, ToolChoice, Vertex};
use httpmock::{Method::POST, MockServer};
use serde_json::json;

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

    let expected_body = json!({
        "contents": [{
            "role": "user",
            "parts": [{ "text": "hi" }]
        }],
        "systemInstruction": {
            "parts": [{ "text": "sys" }]
        },
        "generationConfig": {
            "maxOutputTokens": 12,
            "temperature": 0.2,
            "topP": 0.9,
            "stopSequences": ["done"]
        },
        "tools": [{
            "functionDeclarations": [{
                "name": "add",
                "description": "add"
            }]
        }],
        "toolConfig": {
            "functionCallingConfig": {
                "mode": "ANY",
                "allowedFunctionNames": ["add"]
            }
        }
    });

    let generate_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/models/gemini-pro:generateContent")
                .header("authorization", "Bearer token-abc")
                .json_body_includes(expected_body.to_string());
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
