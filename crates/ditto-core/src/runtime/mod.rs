//! Runtime assembly facade.
//!
//! `catalog` owns static provider/model metadata. `config` owns dynamic user input.
//! `runtime` is the join layer that resolves concrete invocation routes and
//! transport plans from both. Registry snapshots and provider-config semantics
//! live in `crate::runtime_registry` so this module stays focused on assembly.

mod builder_backends;
mod builder_protocol;
mod builtin;
mod explain;
mod model_builders;
mod resolver;
mod route;
mod route_endpoint;
mod route_selection;
#[cfg(feature = "agent")]
pub mod toolbox;
mod transport;

// RUNTIME-BUILTIN-FRONTDOOR: keep the public runtime API on builtin assembly
// entrypoints and result types. Raw catalog-backed planners stay internal so
// `runtime` does not leak `catalog::CatalogRegistry` back out as a second owner.
pub use explain::{
    RuntimeBaseUrlSelectionSource, RuntimeModelSelectionSource, RuntimeProviderSelectionSource,
    RuntimeRouteExplain, explain_builtin_runtime_route,
};
pub use model_builders::{
    build_audio_transcription_model, build_batch_client, build_context_cache_model,
    build_embedding_model, build_file_client, build_image_edit_model, build_image_generation_model,
    build_language_model, build_moderation_model, build_realtime_session_model, build_rerank_model,
    build_speech_model, build_video_generation_model, builtin_runtime_supports_capability,
    builtin_runtime_supports_file_builder,
};
pub use resolver::resolve_builtin_runtime_route;
#[cfg(feature = "agent")]
pub use toolbox::{
    FsToolExecutor, HttpToolExecutor, ShellToolExecutor, TOOL_FS_COPY_FILE, TOOL_FS_DELETE_FILE,
    TOOL_FS_FIND, TOOL_FS_GREP, TOOL_FS_LIST_DIR, TOOL_FS_MKDIR, TOOL_FS_MOVE, TOOL_FS_READ_FILE,
    TOOL_FS_STAT, TOOL_FS_WRITE_FILE, TOOL_HTTP_FETCH, TOOL_SHELL_EXEC, ToolboxExecutor,
    fs_copy_file_tool, fs_delete_file_tool, fs_find_tool, fs_grep_tool, fs_list_dir_tool,
    fs_mkdir_tool, fs_move_tool, fs_read_file_tool, fs_stat_tool, fs_write_file_tool,
    http_fetch_tool, shell_exec_tool, toolbox_tools,
};
pub use transport::{
    RuntimeTransportAuthPlan, RuntimeTransportAuthSelectionSource, RuntimeTransportBaseUrlRewrite,
    RuntimeTransportCredentialSource, RuntimeTransportPlan, RuntimeTransportRequest,
    plan_builtin_runtime_transport,
};
