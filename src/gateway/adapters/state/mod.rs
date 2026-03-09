//! Gateway state adapters.

pub mod file;

use super::super::{RouterConfig, VirtualKeyConfig};

pub use file::{GatewayStateFile, GatewayStateFileError};
