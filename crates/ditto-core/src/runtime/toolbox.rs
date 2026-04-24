//! Host-side agent toolbox runtime.
//!
//! This module owns concrete tool schemas plus filesystem and HTTP executors.
//! `shell_exec` execution lives in `crate::runtime::shell_exec` because it is
//! process runtime, not LLM-facing toolbox semantics.

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("toolbox/tools.rs");
include!("toolbox/safe_fs.rs");
include!("toolbox/fs_tools.rs");
