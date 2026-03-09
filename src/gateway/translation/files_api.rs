impl TranslationBackend {
    async fn resolve_file_client(&self) -> crate::Result<Arc<dyn FileClient>> {
        self.runtime
            .resolve_file_client(self.provider_name(), self.bindings.file_client.as_ref())
            .await
    }

    pub async fn list_files(&self) -> crate::Result<Vec<crate::file::FileObject>> {
        let client = self.resolve_file_client().await?;
        client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> crate::Result<crate::file::FileObject> {
        let client = self.resolve_file_client().await?;
        client.retrieve_file(file_id).await
    }

    pub async fn delete_file(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileDeleteResponse> {
        let client = self.resolve_file_client().await?;
        client.delete_file(file_id).await
    }

    pub async fn download_file_content(
        &self,
        file_id: &str,
    ) -> crate::Result<crate::file::FileContent> {
        let client = self.resolve_file_client().await?;
        client.download_file_content(file_id).await
    }
}
