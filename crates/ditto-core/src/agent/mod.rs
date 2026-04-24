//! Agent module (feature-gated).

pub mod tool_loop;
pub mod types;

pub use tool_loop::{ToolExecutor, ToolLoopAgent};
pub use types::{
    ApprovalHook, StopWhen, ToolApproval, ToolCall, ToolLoopOutcome, ToolLoopState,
    ToolLoopStopReason, ToolResult,
};
