use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileObject {
    pub id: String,
    pub bytes: u64,
    pub created_at: u64,
    pub filename: String,
    pub purpose: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub status_details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDeleteResponse {
    pub id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct FileContent {
    pub bytes: Vec<u8>,
    pub media_type: Option<String>,
}

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

    async fn list_files(&self) -> Result<Vec<FileObject>>;

    async fn retrieve_file(&self, file_id: &str) -> Result<FileObject>;

    async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse>;

    async fn download_file_content(&self, file_id: &str) -> Result<FileContent>;
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

    async fn list_files(&self) -> Result<Vec<FileObject>> {
        self.list_files().await
    }

    async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        self.retrieve_file(file_id).await
    }

    async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        self.delete_file(file_id).await
    }

    async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        self.download_file_content(file_id).await
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

    async fn list_files(&self) -> Result<Vec<FileObject>> {
        self.list_files().await
    }

    async fn retrieve_file(&self, file_id: &str) -> Result<FileObject> {
        self.retrieve_file(file_id).await
    }

    async fn delete_file(&self, file_id: &str) -> Result<FileDeleteResponse> {
        self.delete_file(file_id).await
    }

    async fn download_file_content(&self, file_id: &str) -> Result<FileContent> {
        self.download_file_content(file_id).await
    }
}
