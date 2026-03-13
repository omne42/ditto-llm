//! Gateway backend adapters.

pub mod http;
pub mod proxy;

use super::super::{Backend, GatewayError, GatewayRequest, GatewayResponse};

pub use http::HttpBackend;
pub use proxy::ProxyBackend;
