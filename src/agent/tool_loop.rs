use std::sync::Arc;

use async_trait::async_trait;

use crate::model::LanguageModel;
use crate::types::{ContentPart, Message, Role};
use crate::{DittoError, Result};

use super::types::{
    ApprovalHook, StopWhen, ToolApproval, ToolCall, ToolLoopOutcome, ToolLoopState,
    ToolLoopStopReason, ToolResult,
};

const DEFAULT_MAX_STEPS: usize = 8;

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult>;
}

pub struct ToolLoopAgent<M, E> {
    model: M,
    executor: E,
    max_steps: usize,
    stop_when: Option<Arc<StopWhen>>,
    approval: Option<Arc<ApprovalHook>>,
}

impl<M, E> ToolLoopAgent<M, E>
where
    M: LanguageModel,
    E: ToolExecutor,
{
    pub fn new(model: M, executor: E) -> Self {
        Self {
            model,
            executor,
            max_steps: DEFAULT_MAX_STEPS,
            stop_when: None,
            approval: None,
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_stop_when<F>(mut self, stop_when: F) -> Self
    where
        F: Fn(&ToolLoopState) -> bool + Send + Sync + 'static,
    {
        self.stop_when = Some(Arc::new(stop_when));
        self
    }

    pub fn with_approval<F>(mut self, approval: F) -> Self
    where
        F: Fn(&ToolCall, &ToolLoopState) -> ToolApproval + Send + Sync + 'static,
    {
        self.approval = Some(Arc::new(approval));
        self
    }

    pub async fn run(&self, request: crate::types::GenerateRequest) -> Result<ToolLoopOutcome> {
        if self.max_steps == 0 {
            return Err(DittoError::InvalidResponse(
                "tool loop max_steps must be greater than 0".to_string(),
            ));
        }

        let mut state = ToolLoopState::new(request);
        let mut stop_reason = None;

        for step in 0..self.max_steps {
            state.step = step + 1;

            let response = self.model.generate(state.request.clone()).await?;
            state.last_response = Some(response);
            state.last_tool_calls = collect_tool_calls(
                state
                    .last_response
                    .as_ref()
                    .map(|resp| resp.content.as_slice())
                    .unwrap_or_default(),
            );
            state.last_tool_results.clear();

            if let Some(last_response) = state.last_response.as_ref() {
                if !last_response.content.is_empty() {
                    state.request.messages.push(Message {
                        role: Role::Assistant,
                        content: last_response.content.clone(),
                    });
                }
            }

            if self.should_stop(&state) {
                stop_reason = Some(ToolLoopStopReason::StopWhen);
                break;
            }

            if state.last_tool_calls.is_empty() {
                stop_reason = Some(ToolLoopStopReason::NoToolCalls);
                break;
            }

            for call in &state.last_tool_calls {
                let decision = self.approval_decision(call, &state);
                let result = match decision {
                    ToolApproval::Approve => self.executor.execute(call.clone()).await?,
                    ToolApproval::Deny { reason } => ToolResult {
                        tool_call_id: call.id.clone(),
                        content: if reason.trim().is_empty() {
                            "approval denied".to_string()
                        } else {
                            reason
                        },
                        is_error: Some(true),
                    },
                    ToolApproval::Result(result) => result,
                };
                let result = normalize_result(call, result);
                state.request.messages.push(result.clone().into_message());
                state.last_tool_results.push(result);
            }

            if self.should_stop(&state) {
                stop_reason = Some(ToolLoopStopReason::StopWhen);
                break;
            }
        }

        let stop_reason = stop_reason.unwrap_or(ToolLoopStopReason::MaxSteps);
        Ok(state.into_outcome(stop_reason))
    }

    fn should_stop(&self, state: &ToolLoopState) -> bool {
        self.stop_when
            .as_ref()
            .map(|hook| hook(state))
            .unwrap_or(false)
    }

    fn approval_decision(&self, call: &ToolCall, state: &ToolLoopState) -> ToolApproval {
        self.approval
            .as_ref()
            .map(|hook| hook(call, state))
            .unwrap_or(ToolApproval::Approve)
    }
}

fn collect_tool_calls(parts: &[ContentPart]) -> Vec<ToolCall> {
    parts.iter().filter_map(ToolCall::from_content).collect()
}

fn normalize_result(call: &ToolCall, mut result: ToolResult) -> ToolResult {
    if result.tool_call_id.trim().is_empty() || result.tool_call_id != call.id {
        result.tool_call_id = call.id.clone();
    }
    result
}
