//! Agent module (feature-gated).

pub mod tool_loop;
pub mod toolbox;
pub mod types;

pub use tool_loop::{ToolExecutor, ToolLoopAgent};
pub use toolbox::{
    FsToolExecutor, HttpToolExecutor, ShellToolExecutor, TOOL_FS_LIST_DIR, TOOL_FS_READ_FILE,
    TOOL_FS_STAT, TOOL_FS_WRITE_FILE, TOOL_HTTP_FETCH, TOOL_SHELL_EXEC, ToolboxExecutor,
    fs_list_dir_tool, fs_read_file_tool, fs_stat_tool, fs_write_file_tool, http_fetch_tool,
    shell_exec_tool, toolbox_tools,
};
pub use types::{
    ApprovalHook, StopWhen, ToolApproval, ToolCall, ToolLoopOutcome, ToolLoopState,
    ToolLoopStopReason, ToolResult,
};
