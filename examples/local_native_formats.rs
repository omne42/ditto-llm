use ditto_llm::{Anthropic, GenerateRequest, LanguageModel, Message};
use httpmock::{Method::POST, MockServer};
use serde_json::{Value, json};

#[cfg(feature = "google")]
use ditto_llm::Google;

static PRINT_CLAUDE_REQ: std::sync::Once = std::sync::Once::new();
#[cfg(feature = "google")]
static PRINT_GEMINI_REQ: std::sync::Once = std::sync::Once::new();

fn parse_json_body(req: &httpmock::prelude::HttpMockRequest) -> Option<Value> {
    serde_json::from_slice::<Value>(req.body_ref()).ok()
}

#[cfg(not(feature = "google"))]
fn google_disabled_note() {
    eprintln!(
        "NOTE: google feature is disabled. Re-run with `--features google` to test Gemini format."
    );
}

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    if ditto_llm::utils::test_support::should_skip_httpmock() {
        eprintln!("httpmock disabled in this environment; skipping.");
        return Ok(());
    }

    // This `messages` list is the same unified input you'd send to an OpenAI-compatible model
    // (e.g. glm-4.7). Ditto-LLM will translate it to each provider's native request format.
    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Reply with the single word: ok"),
    ];

    let server = MockServer::start_async().await;

    // --- Claude (Anthropic Messages API) ---
    let anthropic_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/messages")
                .header("anthropic-version", "2023-06-01")
                .header("x-api-key", "dummy")
                .is_true(|req| {
                    let Some(body) = parse_json_body(req) else {
                        return false;
                    };
                    if let Ok(pretty) = serde_json::to_string_pretty(&body) {
                        PRINT_CLAUDE_REQ.call_once(|| {
                            println!("\n=== Claude(Messages) request ===\n{pretty}\n");
                        });
                    }

                    body.get("model").and_then(Value::as_str).is_some()
                        && body.get("messages").and_then(Value::as_array).is_some()
                        && body.get("max_tokens").and_then(Value::as_u64).is_some()
                });

            then.status(200)
                .header("content-type", "application/json")
                .body(
                    json!({
                        "id": "msg_local_123",
                        "content": [{ "type": "text", "text": "ok" }],
                        "stop_reason": "end_turn",
                        "usage": { "input_tokens": 3, "output_tokens": 1 }
                    })
                    .to_string(),
                );
        })
        .await;

    let mut anthropic_request = GenerateRequest::from(messages.clone());
    anthropic_request.max_tokens = Some(16);
    anthropic_request.temperature = Some(0.0);

    let anthropic = Anthropic::new("dummy")
        .with_base_url(server.url("/v1"))
        .with_model("claude-local-test");
    let anthropic_response = anthropic.generate(anthropic_request).await?;
    println!(
        "Claude(Messages) response.text = {}",
        anthropic_response.text()
    );

    // --- Gemini (Google GenAI generateContent) ---
    #[cfg(feature = "google")]
    {
        let google_mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1beta/models/gemini-pro:generateContent")
                    .header("x-goog-api-key", "dummy")
                    .is_true(|req| {
                        let Some(body) = parse_json_body(req) else {
                            return false;
                        };
                        if let Ok(pretty) = serde_json::to_string_pretty(&body) {
                            PRINT_GEMINI_REQ.call_once(|| {
                                println!("\n=== Gemini(generateContent) request ===\n{pretty}\n");
                            });
                        }

                        body.get("contents").and_then(Value::as_array).is_some()
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
                                "promptTokenCount": 3,
                                "candidatesTokenCount": 1,
                                "totalTokenCount": 4
                            }
                        })
                        .to_string(),
                    );
            })
            .await;

        let mut google_request = GenerateRequest::from(messages);
        google_request.max_tokens = Some(16);
        google_request.temperature = Some(0.0);

        let google = Google::new("dummy")
            .with_base_url(server.url("/v1beta"))
            .with_model("gemini-pro");
        let google_response = google.generate(google_request).await?;
        println!(
            "Gemini(generateContent) response.text = {}",
            google_response.text()
        );

        anthropic_mock.assert_async().await;
        google_mock.assert_async().await;
    }

    #[cfg(not(feature = "google"))]
    {
        anthropic_mock.assert_async().await;
        google_disabled_note();
    }

    Ok(())
}
