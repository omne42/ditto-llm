//! Lowest-level support utilities shared across the crate.
//!
//! This layer intentionally avoids provider or gateway semantics. It exists so
//! higher layers can depend on foundational concerns without reaching back into
//! application-specific modules. Configuration parsing stays in `crate::config`
//! and generic helper modules stay under their own owners instead of being
//! re-exported here.

pub mod error;

pub mod secrets;
