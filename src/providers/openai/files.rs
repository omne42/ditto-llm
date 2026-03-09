impl super::OpenAI {
    pub async fn upload_file(
        &self,
        filename: impl Into<String>,
        bytes: Vec<u8>,
    ) -> crate::Result<String> {
        self.upload_file_with_purpose(filename, bytes, "assistants", None)
            .await
    }

    pub async fn upload_file_with_purpose(
        &self,
        filename: impl Into<String>,
        bytes: Vec<u8>,
        purpose: impl Into<String>,
        media_type: Option<&str>,
    ) -> crate::Result<String> {
        self.client
            .upload_file_with_purpose(crate::file::FileUploadRequest {
                filename: filename.into(),
                bytes,
                purpose: purpose.into(),
                media_type: media_type.map(|value| value.to_string()),
            })
            .await
    }

    pub async fn list_files(&self) -> crate::Result<Vec<crate::file::FileObject>> {
        self.client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> crate::Result<crate::file::FileObject> {
        self.client.retrieve_file(file_id).await
    }

    pub async fn delete_file(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileDeleteResponse> {
        self.client.delete_file(file_id).await
    }

    pub async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileContent> {
        self.client.download_file_content(file_id).await
    }
}

#[cfg(feature = "openai")]
#[::async_trait::async_trait]
impl crate::file::FileClient for super::OpenAI {
    fn provider_name(&self) -> &str {
        "openai"
    }

    async fn upload_file_with_purpose(
        &self,
        request: crate::file::FileUploadRequest,
    ) -> crate::Result<String> {
        self.upload_file_with_purpose(
            request.filename,
            request.bytes,
            request.purpose,
            request.media_type.as_deref(),
        )
        .await
    }

    async fn list_files(&self) -> crate::Result<Vec<crate::file::FileObject>> {
        self.list_files().await
    }

    async fn retrieve_file(&self, file_id: &str) -> crate::Result<crate::file::FileObject> {
        self.retrieve_file(file_id).await
    }

    async fn delete_file(&self, file_id: &str) -> crate::Result<crate::file::FileDeleteResponse> {
        self.delete_file(file_id).await
    }

    async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileContent> {
        self.download_file_content(file_id).await
    }
}
