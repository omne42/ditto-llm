#![cfg(all(feature = "gateway", feature = "gateway-translation"))]

// This file is intentionally split to keep each staged Rust file under the pre-commit size limit.
include!("gateway_translation/support.rs");
include!("gateway_translation/tests.rs");
