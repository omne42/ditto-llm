//! Gateway application-layer facade.

#[cfg(feature = "gateway-translation")]
pub mod translation {
    pub use super::super::translation::TranslationBackend;
}
