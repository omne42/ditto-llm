use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::gateway::domain::LocalLruCache;
use ditto_core::capabilities::file::{FileDeleteResponse, FileObject};
use ditto_core::types::{
    Batch, BatchListResponse, VideoDeleteResponse, VideoGenerationResponse, VideoListResponse,
};

use super::response_store::TranslationResponseOwner;

const DEFAULT_TRANSLATION_OWNED_RESOURCE_STORE_MAX_ENTRIES: usize = 1024;
const TRANSLATION_BATCH_HANDLE_PREFIX: &str = "batch_ditto_";
const TRANSLATION_FILE_HANDLE_PREFIX: &str = "file_ditto_";
const TRANSLATION_VIDEO_HANDLE_PREFIX: &str = "video_ditto_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranslationOwnedResourceKind {
    Batch,
    File,
    Video,
}

impl TranslationOwnedResourceKind {
    fn handle_prefix(self) -> &'static str {
        match self {
            Self::Batch => TRANSLATION_BATCH_HANDLE_PREFIX,
            Self::File => TRANSLATION_FILE_HANDLE_PREFIX,
            Self::Video => TRANSLATION_VIDEO_HANDLE_PREFIX,
        }
    }

    fn storage_key(self, scoped_id: &str) -> Option<String> {
        let scoped_id = scoped_id.trim();
        if scoped_id.is_empty() {
            return None;
        }
        Some(format!("{}:{scoped_id}", self.object_name()))
    }

    pub(crate) fn object_name(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::File => "file",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Clone)]
struct StoredOwnedResource {
    owner: TranslationResponseOwner,
    backend_name: String,
    provider_id: String,
    scoped_id: String,
}

#[derive(Clone, Default)]
pub(super) struct TranslationOwnedResourceStore {
    entries: Arc<Mutex<LocalLruCache<StoredOwnedResource>>>,
}

pub(crate) fn scoped_owned_resource_id(
    kind: TranslationOwnedResourceKind,
    backend_name: &str,
    provider_id: &str,
) -> String {
    let backend_name = backend_name.trim();
    let provider_id = provider_id.trim();
    if backend_name.is_empty() || provider_id.is_empty() {
        return provider_id.to_string();
    }

    format!(
        "{}{}_{}_{}",
        kind.handle_prefix(),
        backend_name.len(),
        backend_name,
        provider_id
    )
}

pub(crate) fn scoped_owned_resource_backend_name(
    kind: TranslationOwnedResourceKind,
    scoped_id: &str,
) -> Option<&str> {
    parse_scoped_owned_resource_id(kind, scoped_id).map(|(backend_name, _)| backend_name)
}

fn parse_scoped_owned_resource_id(
    kind: TranslationOwnedResourceKind,
    scoped_id: &str,
) -> Option<(&str, &str)> {
    let rest = scoped_id.trim().strip_prefix(kind.handle_prefix())?;
    let (backend_len, rest) = rest.split_once('_')?;
    let backend_len = backend_len.parse::<usize>().ok()?;
    if backend_len == 0 || rest.len() <= backend_len {
        return None;
    }

    let (backend_name, suffix) = rest.split_at(backend_len);
    let provider_id = suffix.strip_prefix('_')?;
    if backend_name.is_empty() || provider_id.is_empty() {
        return None;
    }

    Some((backend_name, provider_id))
}

impl TranslationOwnedResourceStore {
    async fn track(
        &self,
        kind: TranslationOwnedResourceKind,
        backend_name: &str,
        provider_id: &str,
        owner: TranslationResponseOwner,
    ) -> Option<String> {
        let backend_name = backend_name.trim();
        let provider_id = provider_id.trim();
        if backend_name.is_empty() || provider_id.is_empty() {
            return None;
        }

        let scoped_id = scoped_owned_resource_id(kind, backend_name, provider_id);
        let storage_key = kind.storage_key(&scoped_id)?;

        let mut entries = self.entries.lock().await;
        entries.insert(
            storage_key,
            StoredOwnedResource {
                owner,
                backend_name: backend_name.to_string(),
                provider_id: provider_id.to_string(),
                scoped_id: scoped_id.clone(),
            },
            DEFAULT_TRANSLATION_OWNED_RESOURCE_STORE_MAX_ENTRIES,
        );
        Some(scoped_id)
    }

    async fn resolve_provider_id(
        &self,
        kind: TranslationOwnedResourceKind,
        scoped_id: &str,
        requester: &TranslationResponseOwner,
    ) -> Option<String> {
        let storage_key = kind.storage_key(scoped_id)?;

        self.entries
            .lock()
            .await
            .get(&storage_key)
            .filter(|stored| stored.owner.matches(requester))
            .map(|stored| stored.provider_id)
    }

    async fn visible_ids<'a, I>(
        &self,
        _kind: TranslationOwnedResourceKind,
        backend_name: &str,
        provider_ids: I,
        requester: &TranslationResponseOwner,
    ) -> Vec<(String, String)>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let expected = provider_ids
            .into_iter()
            .map(str::trim)
            .filter(|provider_id| !provider_id.is_empty())
            .collect::<HashSet<_>>();
        if expected.is_empty() {
            return Vec::new();
        }

        self.entries
            .lock()
            .await
            .snapshot()
            .into_iter()
            .filter_map(|(_, stored)| {
                if stored.backend_name != backend_name.trim()
                    || !expected.contains(stored.provider_id.as_str())
                    || !stored.owner.matches(requester)
                {
                    return None;
                }
                Some((stored.provider_id, stored.scoped_id))
            })
            .collect()
    }

    async fn remove(&self, kind: TranslationOwnedResourceKind, scoped_id: &str) -> bool {
        let Some(storage_key) = kind.storage_key(scoped_id) else {
            return false;
        };

        self.entries.lock().await.remove(&storage_key).is_some()
    }
}

impl super::TranslationBackend {
    pub(crate) async fn resolve_owned_resource_id(
        &self,
        kind: TranslationOwnedResourceKind,
        scoped_id: &str,
        requester: &TranslationResponseOwner,
    ) -> Option<String> {
        self.runtime
            .owned_resource_store
            .resolve_provider_id(kind, scoped_id, requester)
            .await
    }

    pub(crate) async fn track_batch_ownership(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        batch: &mut Batch,
    ) {
        if let Some(scoped_id) = self
            .runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::Batch,
                backend_name,
                &batch.id,
                owner.clone(),
            )
            .await
        {
            batch.id = scoped_id;
        }

        if let Some(provider_id) = batch.input_file_id.clone()
            && let Some(scoped_id) = self
                .runtime
                .owned_resource_store
                .track(
                    TranslationOwnedResourceKind::File,
                    backend_name,
                    &provider_id,
                    owner.clone(),
                )
                .await
        {
            batch.input_file_id = Some(scoped_id);
        }
        if let Some(provider_id) = batch.output_file_id.clone()
            && let Some(scoped_id) = self
                .runtime
                .owned_resource_store
                .track(
                    TranslationOwnedResourceKind::File,
                    backend_name,
                    &provider_id,
                    owner.clone(),
                )
                .await
        {
            batch.output_file_id = Some(scoped_id);
        }
        if let Some(provider_id) = batch.error_file_id.clone()
            && let Some(scoped_id) = self
                .runtime
                .owned_resource_store
                .track(
                    TranslationOwnedResourceKind::File,
                    backend_name,
                    &provider_id,
                    owner,
                )
                .await
        {
            batch.error_file_id = Some(scoped_id);
        }
    }

    pub(crate) async fn filter_batch_list_for_owner(
        &self,
        backend_name: &str,
        mut response: BatchListResponse,
        requester: &TranslationResponseOwner,
    ) -> BatchListResponse {
        let visible_batch_ids = self
            .runtime
            .owned_resource_store
            .visible_ids(
                TranslationOwnedResourceKind::Batch,
                backend_name,
                response.batches.iter().map(|batch| batch.id.as_str()),
                requester,
            )
            .await
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();

        let mut filtered = Vec::with_capacity(response.batches.len());
        for mut batch in response.batches {
            if !visible_batch_ids.contains_key(batch.id.as_str()) {
                continue;
            }
            self.track_batch_ownership(requester.clone(), backend_name, &mut batch)
                .await;
            filtered.push(batch);
        }

        response.batches = filtered;
        response.has_more = Some(false);
        response.after = None;
        response
    }

    pub(crate) async fn track_file_id_for_owner(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        provider_id: &str,
    ) -> Option<String> {
        self.runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::File,
                backend_name,
                provider_id,
                owner,
            )
            .await
    }

    pub(crate) async fn track_file_for_owner(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        file: &mut FileObject,
    ) {
        if let Some(scoped_id) = self
            .runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::File,
                backend_name,
                &file.id,
                owner,
            )
            .await
        {
            file.id = scoped_id;
        }
    }

    pub(crate) async fn scope_deleted_file_for_owner(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        deleted: &mut FileDeleteResponse,
    ) {
        if let Some(scoped_id) = self
            .track_file_id_for_owner(owner, backend_name, &deleted.id)
            .await
        {
            deleted.id = scoped_id;
        }
    }

    pub(crate) async fn filter_files_for_owner(
        &self,
        backend_name: &str,
        files: Vec<FileObject>,
        requester: &TranslationResponseOwner,
    ) -> Vec<FileObject> {
        let visible_file_ids = self
            .runtime
            .owned_resource_store
            .visible_ids(
                TranslationOwnedResourceKind::File,
                backend_name,
                files.iter().map(|file| file.id.as_str()),
                requester,
            )
            .await
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();

        let mut filtered = Vec::with_capacity(files.len());
        for mut file in files {
            if !visible_file_ids.contains_key(file.id.as_str()) {
                continue;
            }
            self.track_file_for_owner(requester.clone(), backend_name, &mut file)
                .await;
            filtered.push(file);
        }
        filtered
    }

    pub(crate) async fn remove_owned_file(&self, file_id: &str) {
        let _ = self
            .runtime
            .owned_resource_store
            .remove(TranslationOwnedResourceKind::File, file_id)
            .await;
    }

    pub(crate) async fn track_video_ownership(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        video: &mut VideoGenerationResponse,
    ) {
        if let Some(scoped_id) = self
            .runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::Video,
                backend_name,
                &video.id,
                owner.clone(),
            )
            .await
        {
            video.id = scoped_id;
        }

        if let Some(provider_id) = video.remixed_from_video_id.clone()
            && let Some(scoped_id) = self
                .runtime
                .owned_resource_store
                .track(
                    TranslationOwnedResourceKind::Video,
                    backend_name,
                    &provider_id,
                    owner,
                )
                .await
        {
            video.remixed_from_video_id = Some(scoped_id);
        }
    }

    pub(crate) async fn filter_video_list_for_owner(
        &self,
        backend_name: &str,
        mut response: VideoListResponse,
        requester: &TranslationResponseOwner,
    ) -> VideoListResponse {
        let visible_video_ids = self
            .runtime
            .owned_resource_store
            .visible_ids(
                TranslationOwnedResourceKind::Video,
                backend_name,
                response.videos.iter().map(|video| video.id.as_str()),
                requester,
            )
            .await
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();

        let mut filtered = Vec::with_capacity(response.videos.len());
        for mut video in response.videos {
            if !visible_video_ids.contains_key(video.id.as_str()) {
                continue;
            }
            self.track_video_ownership(requester.clone(), backend_name, &mut video)
                .await;
            filtered.push(video);
        }

        response.videos = filtered;
        response.has_more = Some(false);
        response.after = None;
        response
    }

    pub(crate) async fn scope_deleted_video_for_owner(
        &self,
        owner: TranslationResponseOwner,
        backend_name: &str,
        deleted: &mut VideoDeleteResponse,
    ) {
        if let Some(scoped_id) = self
            .runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::Video,
                backend_name,
                &deleted.id,
                owner,
            )
            .await
        {
            deleted.id = scoped_id;
        }
    }

    pub(crate) async fn remove_owned_video(&self, video_id: &str) {
        let _ = self
            .runtime
            .owned_resource_store
            .remove(TranslationOwnedResourceKind::Video, video_id)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures_util::StreamExt;

    use ditto_core::contracts::{FinishReason, GenerateRequest, GenerateResponse, Usage};
    use ditto_core::llm_core::model::{LanguageModel, StreamResult};

    #[derive(Clone)]
    struct NoopModel;

    #[async_trait]
    impl LanguageModel for NoopModel {
        fn provider(&self) -> &str {
            "fake"
        }

        fn model_id(&self) -> &str {
            "fake-model"
        }

        async fn generate(
            &self,
            _request: GenerateRequest,
        ) -> ditto_core::error::Result<GenerateResponse> {
            Ok(GenerateResponse {
                content: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                warnings: Vec::new(),
                provider_metadata: None,
            })
        }

        async fn stream(
            &self,
            _request: GenerateRequest,
        ) -> ditto_core::error::Result<StreamResult> {
            Ok(futures_util::stream::empty().boxed())
        }
    }

    fn owner(virtual_key_id: &str) -> TranslationResponseOwner {
        TranslationResponseOwner {
            virtual_key_id: Some(virtual_key_id.to_string()),
            tenant_id: Some("tenant-a".to_string()),
            ..TranslationResponseOwner::default()
        }
    }

    #[tokio::test]
    async fn batch_tracking_scopes_batch_and_related_file_ids() {
        let backend = super::super::TranslationBackend::new("fake", Arc::new(NoopModel));
        let mut batch = Batch {
            id: "batch-1".to_string(),
            input_file_id: Some("file-input".to_string()),
            output_file_id: Some("file-output".to_string()),
            error_file_id: Some("file-error".to_string()),
            ..Batch::default()
        };

        backend
            .track_batch_ownership(owner("vk-1"), "primary", &mut batch)
            .await;

        assert_eq!(batch.id, "batch_ditto_7_primary_batch-1");
        assert_eq!(
            batch.input_file_id.as_deref(),
            Some("file_ditto_7_primary_file-input")
        );
        assert_eq!(
            batch.output_file_id.as_deref(),
            Some("file_ditto_7_primary_file-output")
        );
        assert_eq!(
            batch.error_file_id.as_deref(),
            Some("file_ditto_7_primary_file-error")
        );
        assert_eq!(
            backend
                .resolve_owned_resource_id(
                    TranslationOwnedResourceKind::Batch,
                    &batch.id,
                    &owner("vk-1"),
                )
                .await
                .as_deref(),
            Some("batch-1")
        );
        assert!(
            backend
                .resolve_owned_resource_id(
                    TranslationOwnedResourceKind::File,
                    batch.input_file_id.as_deref().expect("scoped file id"),
                    &owner("vk-2"),
                )
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn list_filters_are_fail_closed_and_rewrite_ids() {
        let backend = super::super::TranslationBackend::new("fake", Arc::new(NoopModel));
        let requester = owner("vk-1");

        let _ = backend
            .track_file_id_for_owner(requester.clone(), "primary", "file-1")
            .await;
        let _ = backend
            .track_file_id_for_owner(requester.clone(), "secondary", "file-2")
            .await;

        let files = backend
            .filter_files_for_owner(
                "primary",
                vec![
                    FileObject {
                        id: "file-1".to_string(),
                        bytes: 1,
                        created_at: 0,
                        filename: "a.txt".to_string(),
                        purpose: "assistants".to_string(),
                        status: None,
                        status_details: None,
                    },
                    FileObject {
                        id: "file-2".to_string(),
                        bytes: 1,
                        created_at: 0,
                        filename: "b.txt".to_string(),
                        purpose: "assistants".to_string(),
                        status: None,
                        status_details: None,
                    },
                ],
                &requester,
            )
            .await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, "file_ditto_7_primary_file-1");

        let _ = backend
            .runtime
            .owned_resource_store
            .track(
                TranslationOwnedResourceKind::Video,
                "primary",
                "video-1",
                requester.clone(),
            )
            .await;

        let videos = backend
            .filter_video_list_for_owner(
                "primary",
                VideoListResponse {
                    videos: vec![
                        VideoGenerationResponse {
                            id: "video-1".to_string(),
                            ..VideoGenerationResponse::default()
                        },
                        VideoGenerationResponse {
                            id: "video-2".to_string(),
                            ..VideoGenerationResponse::default()
                        },
                    ],
                    has_more: Some(true),
                    after: Some("video-2".to_string()),
                    ..VideoListResponse::default()
                },
                &requester,
            )
            .await;
        assert_eq!(videos.videos.len(), 1);
        assert_eq!(videos.videos[0].id, "video_ditto_7_primary_video-1");
        assert_eq!(videos.has_more, Some(false));
        assert_eq!(videos.after, None);
    }
}
