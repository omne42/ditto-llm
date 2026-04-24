//! Ditto-internal stream bootstrap helpers.
//!
//! Generic SSE parsing and websocket URL rewrite live in lower layers
//! (`http-kit` and `runtime`). This module only keeps Ditto's stream bootstrap
//! glue for warning prelude chunks and crate-local stream assembly.

mod streaming;

#[allow(unused_imports)]
pub(crate) use streaming::{init_data_stream, init_sse_stream};
