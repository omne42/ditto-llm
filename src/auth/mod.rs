//! Auth module (feature-gated).

pub mod oauth;
pub mod sigv4;

pub use oauth::{OAuthClientCredentials, OAuthToken};
pub use sigv4::{SigV4Headers, SigV4Signer, SigV4SigningResult, SigV4Timestamp};
