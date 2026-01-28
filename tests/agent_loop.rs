#![cfg(feature = "agent")]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use serde_json::json;

use ditto_llm::agent::{
    ToolApproval, ToolCall, ToolExecutor, ToolLoopAgent, ToolLoopStopReason, ToolResult,
};
use ditto_llm::{
    ContentPart, DittoError, FinishReason, GenerateRequest, GenerateResponse, LanguageModel,
    Message, Result, Role, StreamResult, Tool,
};

fn lock_or_err<'a, T>(mutex: &'a Mutex<T>, context: &str) -> Result<MutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|_| DittoError::InvalidResponse(format!("{context} lock poisoned")))
}

#[derive(Clone)]
struct StubModel {
    responses: Arc<Mutex<VecDeque<GenerateResponse>>>,
    requests: Arc<Mutex<Vec<GenerateRequest>>>,
}

impl StubModel {
    fn new(responses: Vec<GenerateResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Arc<Mutex<Vec<GenerateRequest>>> {
        self.requests.clone()
    }
}

#[async_trait]
impl LanguageModel for StubModel {
    fn provider(&self) -> &str {
        "stub"
    }

    fn model_id(&self) -> &str {
        "stub-model"
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        lock_or_err(&self.requests, "stub model requests")?.push(request);
        let mut responses = lock_or_err(&self.responses, "stub model responses")?;
        responses.pop_front().ok_or_else(|| {
            DittoError::InvalidResponse("stub model has no responses left".to_string())
        })
    }

    async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
        Err(DittoError::InvalidResponse(
            "stub model does not support stream".to_string(),
        ))
    }
}

#[derive(Clone, Default)]
struct StubToolExecutor {
    calls: Arc<Mutex<Vec<ToolCall>>>,
}

impl StubToolExecutor {
    fn calls(&self) -> Arc<Mutex<Vec<ToolCall>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl ToolExecutor for StubToolExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        lock_or_err(&self.calls, "stub tool calls")?.push(call.clone());
        let a = call
            .arguments
            .get("a")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let b = call
            .arguments
            .get("b")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(ToolResult {
            tool_call_id: call.id,
            content: json!({ "result": a + b }).to_string(),
            is_error: None,
        })
    }
}

fn tool_call_response(id: &str, name: &str, args: serde_json::Value) -> GenerateResponse {
    GenerateResponse {
        content: vec![ContentPart::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        }],
        finish_reason: FinishReason::ToolCalls,
        ..GenerateResponse::default()
    }
}

fn text_response(text: &str) -> GenerateResponse {
    GenerateResponse {
        content: vec![ContentPart::Text {
            text: text.to_string(),
        }],
        finish_reason: FinishReason::Stop,
        ..GenerateResponse::default()
    }
}

fn basic_request() -> GenerateRequest {
    let tools = vec![Tool {
        name: "add".to_string(),
        description: Some("Add two numbers".to_string()),
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

    let mut request: GenerateRequest = vec![Message::user("compute")].into();
    request.tools = Some(tools);
    request
}

#[tokio::test]
async fn tool_loop_executes_tools() -> Result<()> {
    let responses = vec![
        tool_call_response("call_1", "add", json!({"a": 1, "b": 2})),
        text_response("done"),
    ];

    let model = StubModel::new(responses);
    let requests = model.requests();
    let executor = StubToolExecutor::default();
    let calls = executor.calls();

    let agent = ToolLoopAgent::new(model, executor).with_max_steps(4);
    let outcome = agent.run(basic_request()).await?;

    assert_eq!(outcome.stop_reason, ToolLoopStopReason::NoToolCalls);
    assert_eq!(outcome.steps, 2);
    assert_eq!(
        outcome.last_response.as_ref().map(|r| r.text()),
        Some("done".to_string())
    );

    let calls = lock_or_err(&calls, "stub tool calls")?;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");

    let requests = lock_or_err(&requests, "stub model requests")?;
    assert_eq!(requests.len(), 2);
    let second_messages = &requests[1].messages;
    assert!(second_messages.iter().any(|m| m.role == Role::Tool));

    let tool_messages: Vec<_> = outcome
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect();
    assert_eq!(tool_messages.len(), 1);
    match &tool_messages[0].content[0] {
        ContentPart::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, "call_1");
            assert!(content.contains("3"));
            assert_eq!(*is_error, None);
        }
        _ => {
            return Err(DittoError::InvalidResponse(
                "expected tool result content".to_string(),
            ));
        }
    }

    Ok(())
}

#[tokio::test]
async fn approval_denied_skips_executor() -> Result<()> {
    let responses = vec![
        tool_call_response("call_1", "add", json!({"a": 4, "b": 2})),
        text_response("blocked"),
    ];

    let model = StubModel::new(responses);
    let executor = StubToolExecutor::default();
    let calls = executor.calls();

    let agent =
        ToolLoopAgent::new(model, executor).with_approval(|_call, _state| ToolApproval::Deny {
            reason: "nope".to_string(),
        });
    let outcome = agent.run(basic_request()).await?;

    let calls = lock_or_err(&calls, "stub tool calls")?;
    assert!(calls.is_empty());

    let denied = outcome.messages.iter().find(|m| m.role == Role::Tool);
    let Some(denied) = denied else {
        return Err(DittoError::InvalidResponse(
            "missing denied tool result message".to_string(),
        ));
    };
    match &denied.content[0] {
        ContentPart::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(content, "nope");
            assert_eq!(*is_error, Some(true));
        }
        _ => {
            return Err(DittoError::InvalidResponse(
                "expected tool result content".to_string(),
            ));
        }
    }

    Ok(())
}

#[tokio::test]
async fn stop_when_triggers_before_tools() -> Result<()> {
    let responses = vec![tool_call_response(
        "call_1",
        "add",
        json!({"a": 10, "b": 1}),
    )];

    let model = StubModel::new(responses);
    let executor = StubToolExecutor::default();
    let calls = executor.calls();

    let agent = ToolLoopAgent::new(model, executor)
        .with_max_steps(3)
        .with_stop_when(|state| state.step >= 1);
    let outcome = agent.run(basic_request()).await?;

    assert_eq!(outcome.stop_reason, ToolLoopStopReason::StopWhen);
    assert_eq!(outcome.steps, 1);

    let calls = lock_or_err(&calls, "stub tool calls")?;
    assert!(calls.is_empty());

    Ok(())
}
