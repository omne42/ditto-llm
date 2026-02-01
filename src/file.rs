use async_trait::async_trait;

use crate::Result;

#[derive(Debug, Clone)]
pub struct FileUploadRequest {
    pub filename: String,
    pub bytes: Vec<u8>,
    pub purpose: String,
    pub media_type: Option<String>,
}

#[async_trait]
pub trait FileClient: Send + Sync {
    fn provider(&self) -> &str;

    async fn upload_file_with_purpose(&self, request: FileUploadRequest) -> Result<String>;
}

#[cfg(feature = "openai")]
#[async_trait]
impl FileClient for crate::providers::OpenAI {
    fn provider(&self) -> &str {
        "openai"
    }

    async fn upload_file_with_purpose(&self, request: FileUploadRequest) -> Result<String> {
        self.upload_file_with_purpose(
            request.filename,
            request.bytes,
            request.purpose,
            request.media_type.as_deref(),
        )
        .await
    }
}

#[cfg(feature = "openai-compatible")]
#[async_trait]
impl FileClient for crate::providers::OpenAICompatible {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    async fn upload_file_with_purpose(&self, request: FileUploadRequest) -> Result<String> {
        self.upload_file_with_purpose(
            request.filename,
            request.bytes,
            request.purpose,
            request.media_type.as_deref(),
        )
        .await
    }
}
