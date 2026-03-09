use std::time::Duration;

use async_trait::async_trait;

use crate::DittoError;
use crate::Result;
use crate::file::FileContent;
use crate::types::{
    VideoContentVariant, VideoDeleteResponse, VideoGenerationRequest, VideoGenerationResponse,
    VideoListRequest, VideoListResponse, VideoRemixRequest,
};

#[async_trait]
pub trait VideoGenerationModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn create(&self, request: VideoGenerationRequest) -> Result<VideoGenerationResponse>;

    async fn retrieve(&self, video_id: &str) -> Result<VideoGenerationResponse>;

    async fn list(&self, request: VideoListRequest) -> Result<VideoListResponse> {
        let _ = request;
        Err(DittoError::InvalidResponse(format!(
            "video listing is not supported by provider={} model={}",
            self.provider(),
            self.model_id()
        )))
    }

    async fn delete(&self, video_id: &str) -> Result<VideoDeleteResponse> {
        let _ = video_id;
        Err(DittoError::InvalidResponse(format!(
            "video deletion is not supported by provider={} model={}",
            self.provider(),
            self.model_id()
        )))
    }

    async fn download_content(
        &self,
        video_id: &str,
        variant: Option<VideoContentVariant>,
    ) -> Result<FileContent> {
        let _ = video_id;
        let _ = variant;
        Err(DittoError::InvalidResponse(format!(
            "video content download is not supported by provider={} model={}",
            self.provider(),
            self.model_id()
        )))
    }

    async fn remix(
        &self,
        video_id: &str,
        request: VideoRemixRequest,
    ) -> Result<VideoGenerationResponse> {
        let _ = video_id;
        let _ = request;
        Err(DittoError::InvalidResponse(format!(
            "video remix is not supported by provider={} model={}",
            self.provider(),
            self.model_id()
        )))
    }

    async fn create_and_poll(
        &self,
        request: VideoGenerationRequest,
        poll_interval: Duration,
    ) -> Result<VideoGenerationResponse> {
        let video = self.create(request).await?;
        self.poll_until_terminal(&video.id, poll_interval, None)
            .await
    }

    async fn poll_until_terminal(
        &self,
        video_id: &str,
        poll_interval: Duration,
        max_attempts: Option<usize>,
    ) -> Result<VideoGenerationResponse> {
        let pause = if poll_interval.is_zero() {
            Duration::from_secs(1)
        } else {
            poll_interval
        };
        let mut attempts = 0usize;

        loop {
            let video = self.retrieve(video_id).await?;
            if video.status.is_terminal() {
                return Ok(video);
            }

            attempts = attempts.saturating_add(1);
            if max_attempts.is_some_and(|limit| attempts >= limit) {
                return Ok(video);
            }

            tokio::time::sleep(pause).await;
        }
    }
}
