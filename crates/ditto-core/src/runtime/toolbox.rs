//! Host-side agent toolbox runtime.
//!
//! This module owns concrete tool schemas plus filesystem, shell, and HTTP
//! executors. It stays under `runtime` so `agent` only exposes the protocol and
//! loop orchestration needed by LLM-facing callers.

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("toolbox/tools.rs");
include!("toolbox/safe_fs.rs");
include!("toolbox/fs_tools.rs");
