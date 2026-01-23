use thiserror::Error;

#[derive(Debug, Error)]
pub enum DittoError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("failed to run auth command: {0}")]
    AuthCommand(String),
    #[error("failed to parse json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DittoError>;
