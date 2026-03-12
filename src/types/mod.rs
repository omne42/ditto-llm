//! Protocol payload types.
//!
//! This namespace owns modality-specific payload DTOs only.
//! Provider passthrough policy stays in `crate::provider_options`; shared LLM
//! call contracts and outcome semantics live under `crate::contracts`.
// TYPES-PAYLOAD-ONLY: non-LLM protocol DTOs stay here; canonical LLM call
// contracts stay under `crate::contracts`.

mod audio;
mod batch;
#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
mod generate_request_support;
mod image;
mod moderation;
mod rerank;
mod video;
pub use audio::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
    SpeechResponseFormat, TranscriptionResponseFormat,
};
pub use batch::{
    Batch, BatchCreateRequest, BatchListResponse, BatchRequestCounts, BatchResponse, BatchStatus,
};
pub use image::{
    ImageEditRequest, ImageEditResponse, ImageEditUpload, ImageGenerationRequest,
    ImageGenerationResponse, ImageResponseFormat, VideoReferenceUpload,
};
pub use moderation::{ModerationInput, ModerationRequest, ModerationResponse, ModerationResult};
pub use rerank::{RerankDocument, RerankRequest, RerankResponse, RerankResult};
pub use video::{
    VideoContentVariant, VideoDeleteResponse, VideoGenerationError, VideoGenerationRequest,
    VideoGenerationResponse, VideoGenerationStatus, VideoListOrder, VideoListRequest,
    VideoListResponse, VideoRemixRequest,
};

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
#[doc(hidden)]
pub use generate_request_support::{
    GenerateRequestSupport, warn_unsupported_generate_request_options,
};
