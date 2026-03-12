#[::async_trait::async_trait]
impl crate::capabilities::file::FileClient for OpenAICompatible {
    fn provider_name(&self) -> &str {
        "openai-compatible"
    }

    async fn upload_file_with_purpose(
        &self,
        request: crate::capabilities::file::FileUploadRequest,
    ) -> crate::foundation::error::Result<String> {
        self.upload_file_with_purpose(
            request.filename,
            request.bytes,
            request.purpose,
            request.media_type.as_deref(),
        )
        .await
    }

    async fn list_files(&self) -> crate::foundation::error::Result<Vec<crate::capabilities::file::FileObject>> {
        self.list_files().await
    }

    async fn retrieve_file(
        &self,
        file_id: &str,
    ) -> crate::foundation::error::Result<crate::capabilities::file::FileObject> {
        self.retrieve_file(file_id).await
    }

    async fn delete_file(
        &self,
        file_id: &str,
    ) -> crate::foundation::error::Result<crate::capabilities::file::FileDeleteResponse> {
        self.delete_file(file_id).await
    }

    async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::foundation::error::Result<crate::capabilities::file::FileContent> {
        self.download_file_content(file_id).await
    }
}
