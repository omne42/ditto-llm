use serde_json::Value;

use crate::types::{ContentPart, GenerateRequest, GenerateResponse, Message, Role};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl ToolCall {
    pub fn from_content(part: &ContentPart) -> Option<Self> {
        match part {
            ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => Some(Self {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: Option<bool>,
}

impl ToolResult {
    pub fn into_message(self) -> Message {
        Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: self.tool_call_id,
                content: self.content,
                is_error: self.is_error,
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolApproval {
    Approve,
    Deny { reason: String },
    Result(ToolResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLoopStopReason {
    StopWhen,
    MaxSteps,
    NoToolCalls,
}

#[derive(Debug, Clone)]
pub struct ToolLoopOutcome {
    pub messages: Vec<Message>,
    pub last_response: Option<GenerateResponse>,
    pub steps: usize,
    pub stop_reason: ToolLoopStopReason,
}

#[derive(Debug, Clone)]
pub struct ToolLoopState {
    pub step: usize,
    pub request: GenerateRequest,
    pub last_response: Option<GenerateResponse>,
    pub last_tool_calls: Vec<ToolCall>,
    pub last_tool_results: Vec<ToolResult>,
}

impl ToolLoopState {
    pub fn new(request: GenerateRequest) -> Self {
        Self {
            step: 0,
            request,
            last_response: None,
            last_tool_calls: Vec::new(),
            last_tool_results: Vec::new(),
        }
    }

    pub fn into_outcome(self, stop_reason: ToolLoopStopReason) -> ToolLoopOutcome {
        ToolLoopOutcome {
            messages: self.request.messages,
            last_response: self.last_response,
            steps: self.step,
            stop_reason,
        }
    }
}

pub type StopWhen = dyn Fn(&ToolLoopState) -> bool + Send + Sync;
pub type ApprovalHook = dyn Fn(&ToolCall, &ToolLoopState) -> ToolApproval + Send + Sync;
