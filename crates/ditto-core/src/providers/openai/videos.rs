use async_trait::async_trait;

use crate::providers::openai_like;
use crate::providers::openai_videos_common;

use crate::capabilities::file::FileContent;
use crate::capabilities::video::VideoGenerationModel;
use crate::config::{Env, ProviderConfig};
use crate::foundation::error::Result;
use crate::types::{
    VideoContentVariant, VideoDeleteResponse, VideoGenerationRequest, VideoGenerationResponse,
    VideoListRequest, VideoListResponse, VideoRemixRequest,
};

#[derive(Clone)]
pub struct OpenAIVideos {
    client: openai_like::OpenAiLikeClient,
}

impl OpenAIVideos {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: openai_like::OpenAiLikeClient::new(api_key),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.client = self.client.with_http_client(http);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client = self.client.with_base_url(base_url);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.client = self.client.with_model(model);
        self
    }

    pub fn with_max_binary_response_bytes(mut self, max_bytes: usize) -> Self {
        self.client = self.client.with_max_binary_response_bytes(max_bytes);
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY"];
        Ok(Self {
            client: openai_like::OpenAiLikeClient::from_config_required(config, env, DEFAULT_KEYS)
                .await?,
        })
    }
}

#[async_trait]
impl VideoGenerationModel for OpenAIVideos {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.client.model.as_str()
    }

    async fn create(&self, request: VideoGenerationRequest) -> Result<VideoGenerationResponse> {
        openai_videos_common::create(
            self.provider(),
            &self.client,
            Some(self.client.model.as_str()),
            request,
        )
        .await
    }

    async fn retrieve(&self, video_id: &str) -> Result<VideoGenerationResponse> {
        openai_videos_common::retrieve(&self.client, video_id).await
    }

    async fn list(&self, request: VideoListRequest) -> Result<VideoListResponse> {
        openai_videos_common::list(&self.client, request).await
    }

    async fn delete(&self, video_id: &str) -> Result<VideoDeleteResponse> {
        openai_videos_common::delete(&self.client, video_id).await
    }

    async fn download_content(
        &self,
        video_id: &str,
        variant: Option<VideoContentVariant>,
    ) -> Result<FileContent> {
        openai_videos_common::download_content(&self.client, video_id, variant).await
    }

    async fn remix(
        &self,
        video_id: &str,
        request: VideoRemixRequest,
    ) -> Result<VideoGenerationResponse> {
        openai_videos_common::remix(self.provider(), &self.client, video_id, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{
        Method::{DELETE, GET, POST},
        MockServer,
    };

    #[tokio::test]
    async fn create_video_posts_to_videos_endpoint() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/videos")
                    .body_includes("road at dusk")
                    .body_includes("sora-2");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "vid_123",
                            "object": "video",
                            "status": "queued",
                            "model": "sora-2",
                            "prompt": "road at dusk",
                            "progress": 0,
                            "seconds": "4",
                            "size": "1280x720",
                            "created_at": 123,
                            "error": null
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIVideos::new("")
            .with_base_url(server.url("/v1"))
            .with_model("sora-2");

        let response = client
            .create(VideoGenerationRequest {
                prompt: "road at dusk".to_string(),
                input_reference: None,
                model: None,
                seconds: Some(4),
                size: Some("1280x720".to_string()),
                provider_options: None,
            })
            .await?;

        mock.assert_async().await;
        assert_eq!(response.id, "vid_123");
        assert_eq!(response.status, crate::types::VideoGenerationStatus::Queued);
        assert_eq!(response.model.as_deref(), Some("sora-2"));
        assert_eq!(response.seconds.as_deref(), Some("4"));
        Ok(())
    }

    #[tokio::test]
    async fn list_and_delete_videos_follow_resource_routes() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let list_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/videos")
                    .query_param("limit", "1")
                    .query_param("order", "desc");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "data": [{
                                "id": "vid_123",
                                "object": "video",
                                "status": "completed",
                                "model": "sora-2",
                                "prompt": "road at dusk"
                            }],
                            "has_more": false,
                            "last_id": "vid_123"
                        })
                        .to_string(),
                    );
            })
            .await;
        let delete_mock = server
            .mock_async(|when, then| {
                when.method(DELETE).path("/v1/videos/vid_123");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "vid_123",
                            "deleted": true,
                            "object": "video.deleted"
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAIVideos::new("").with_base_url(server.url("/v1"));
        let list = client
            .list(VideoListRequest {
                limit: Some(1),
                after: None,
                order: Some(crate::types::VideoListOrder::Desc),
            })
            .await?;
        let deleted = client.delete("vid_123").await?;

        list_mock.assert_async().await;
        delete_mock.assert_async().await;
        assert_eq!(list.videos.len(), 1);
        assert_eq!(list.after.as_deref(), Some("vid_123"));
        assert!(deleted.deleted);
        Ok(())
    }

    #[tokio::test]
    async fn download_video_content_passes_variant_query() -> Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/videos/vid_123/content")
                    .query_param("variant", "thumbnail");
                then.status(200)
                    .header("content-type", "image/jpeg")
                    .body(vec![1u8, 2, 3, 4]);
            })
            .await;

        let client = OpenAIVideos::new("").with_base_url(server.url("/v1"));
        let content = client
            .download_content(
                "vid_123",
                Some(crate::types::VideoContentVariant::Thumbnail),
            )
            .await?;

        mock.assert_async().await;
        assert_eq!(content.bytes, vec![1u8, 2, 3, 4]);
        assert_eq!(content.media_type.as_deref(), Some("image/jpeg"));
        Ok(())
    }
}
