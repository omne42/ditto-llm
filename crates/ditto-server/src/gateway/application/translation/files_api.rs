use std::sync::Arc;

use bytes::Bytes;
use serde_json::{Map, Value};

use ditto_core::capabilities::file::{
    FileClient, FileDeleteResponse, FileObject, FileUploadRequest,
};

use super::{ParseResult, TranslationBackend};

pub fn files_upload_request_to_request(
    content_type: &str,
    body: &Bytes,
) -> ParseResult<FileUploadRequest> {
    let mut file: Option<crate::gateway::multipart::MultipartPart> = None;
    let mut purpose: Option<String> = None;

    let parts = crate::gateway::multipart::parse_multipart_form(content_type, body)?;
    for part in parts {
        match part.name.as_str() {
            "file" => file = Some(part),
            "purpose" => {
                let value = String::from_utf8_lossy(part.data.as_ref())
                    .trim()
                    .to_string();
                if !value.is_empty() {
                    purpose = Some(value);
                }
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| "files request missing file".to_string())?;
    let purpose = purpose.ok_or_else(|| "files request missing purpose".to_string())?;
    let filename = file
        .filename
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "file".to_string());

    Ok(FileUploadRequest {
        filename,
        bytes: file.data.to_vec(),
        purpose,
        media_type: file.content_type.clone(),
    })
}

pub fn file_upload_response_to_openai(
    file_id: &str,
    filename: String,
    purpose: String,
    bytes: usize,
    created_at: u64,
) -> Value {
    serde_json::json!({
        "id": file_id,
        "object": "file",
        "bytes": bytes,
        "created_at": created_at,
        "filename": filename,
        "purpose": purpose,
    })
}

pub fn file_to_openai(file: &FileObject) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert("id".to_string(), Value::String(file.id.clone()));
    out.insert("object".to_string(), Value::String("file".to_string()));
    out.insert("bytes".to_string(), Value::Number(file.bytes.into()));
    out.insert(
        "created_at".to_string(),
        Value::Number(file.created_at.into()),
    );
    out.insert("filename".to_string(), Value::String(file.filename.clone()));
    out.insert("purpose".to_string(), Value::String(file.purpose.clone()));
    if let Some(status) = file.status.as_deref() {
        out.insert("status".to_string(), Value::String(status.to_string()));
    }
    if let Some(details) = file.status_details.clone() {
        out.insert("status_details".to_string(), details);
    }
    Value::Object(out)
}

pub fn file_list_response_to_openai(files: &[FileObject]) -> Value {
    Value::Object(Map::from_iter([
        ("object".to_string(), Value::String("list".to_string())),
        (
            "data".to_string(),
            Value::Array(files.iter().map(file_to_openai).collect()),
        ),
    ]))
}

pub fn file_delete_response_to_openai(response: &FileDeleteResponse) -> Value {
    serde_json::json!({
        "id": response.id,
        "object": "file",
        "deleted": response.deleted,
    })
}

impl TranslationBackend {
    pub(super) async fn resolve_file_client(
        &self,
    ) -> ditto_core::error::Result<Arc<dyn FileClient>> {
        self.runtime
            .resolve_file_client(self.provider_name(), self.bindings.file_client.as_ref())
            .await
    }

    pub async fn list_files(&self) -> ditto_core::error::Result<Vec<FileObject>> {
        let client = self.resolve_file_client().await?;
        client.list_files().await
    }

    pub async fn retrieve_file(&self, file_id: &str) -> ditto_core::error::Result<FileObject> {
        let client = self.resolve_file_client().await?;
        client.retrieve_file(file_id).await
    }

    pub async fn delete_file(
        &self,
        file_id: &str,
    ) -> ditto_core::error::Result<FileDeleteResponse> {
        let client = self.resolve_file_client().await?;
        client.delete_file(file_id).await
    }

    pub async fn download_file_content(
        &self,
        file_id: &str,
    ) -> ditto_core::error::Result<ditto_core::capabilities::file::FileContent> {
        let client = self.resolve_file_client().await?;
        client.download_file_content(file_id).await
    }
}
