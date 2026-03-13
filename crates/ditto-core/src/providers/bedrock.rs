// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("bedrock/client.rs");
include!("bedrock/messages_api.rs");
#[cfg(feature = "streaming")]
include!("bedrock/eventstream.rs");
include!("bedrock/tests.rs");
