use ditto_llm::{ContentPart, GenerateRequest, LanguageModel, Message, OpenAI, Tool, ToolChoice};
use serde_json::json;

fn add(arguments: &serde_json::Value) -> serde_json::Value {
    let a = arguments.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
    let b = arguments.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
    json!({ "result": a + b })
}

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".to_string())
    })?;
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let openai = OpenAI::new(api_key).with_model(model);

    let tools = vec![Tool {
        name: "add".to_string(),
        description: Some("Add two integers.".to_string()),
        parameters: json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "integer" }
            },
            "required": ["a", "b"]
        }),
        strict: Some(true),
    }];

    let request = GenerateRequest {
        messages: vec![
            Message::system("You are a helpful assistant. Use tools when appropriate."),
            Message::user("Compute 40 + 2 using the add tool."),
        ],
        tools: Some(tools),
        tool_choice: Some(ToolChoice::Required),
        ..GenerateRequest::from(Vec::new())
    };

    let response = openai.generate(request).await?;

    let mut tool_calls = Vec::new();
    for part in &response.content {
        if let ContentPart::ToolCall {
            id,
            name,
            arguments,
        } = part
        {
            tool_calls.push((id.clone(), name.clone(), arguments.clone()));
        }
    }

    if tool_calls.is_empty() {
        println!("model did not call a tool; text: {}", response.text());
        return Ok(());
    }

    let mut followup_messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Compute 40 + 2 using the add tool."),
    ];

    for (tool_call_id, tool_name, args) in tool_calls {
        let output = match tool_name.as_str() {
            "add" => add(&args),
            _ => json!({ "error": "unknown tool" }),
        };

        followup_messages.push(Message {
            role: ditto_llm::Role::Assistant,
            content: vec![ContentPart::ToolCall {
                id: tool_call_id.clone(),
                name: tool_name,
                arguments: args,
            }],
        });
        followup_messages.push(Message::tool_result(tool_call_id, output.to_string()));
    }

    let response = openai.generate(followup_messages.into()).await?;
    println!("{}", response.text());

    Ok(())
}
