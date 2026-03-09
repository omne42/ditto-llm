#[::async_trait::async_trait]
impl crate::file::FileClient for OpenAICompatible {
    fn provider_name(&self) -> &str {
        "openai-compatible"
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

    async fn delete_file(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileDeleteResponse> {
        self.delete_file(file_id).await
    }

    async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileContent> {
        self.download_file_content(file_id).await
    }
}
