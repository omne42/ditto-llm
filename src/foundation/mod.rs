//! Lowest-level support utilities shared across the crate.
//!
//! This layer intentionally avoids provider or gateway semantics. It exists so
//! higher layers can depend on foundational concerns without reaching back into
//! application-specific modules.

pub mod env {
    pub use crate::config::{Env, parse_dotenv};
}

pub mod error {
    pub use crate::error::{DittoError, ProviderResolutionError, Result};
}

pub mod secrets;

pub mod utils {
    pub use crate::utils::*;
}
