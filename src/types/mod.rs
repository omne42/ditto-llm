use serde_json::Value;

use crate::Result;

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
mod llm;
mod moderation;
mod provider_options_envelope;
#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
mod provider_options_support;
mod rerank;
mod tool_call;
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
pub use llm::{
    ContentPart, FileSource, FinishReason, GenerateRequest, GenerateResponse, ImageSource,
    JsonSchemaFormat, Message, ProviderOptions, ReasoningEffort, ReasoningSummary, ResponseFormat,
    Role, StreamChunk, Tool, ToolChoice, Usage, Warning,
};
pub use moderation::{ModerationInput, ModerationRequest, ModerationResponse, ModerationResult};
pub use provider_options_envelope::ProviderOptionsEnvelope;
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
#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
pub(crate) use provider_options_support::{
    ProviderOptionsSupport, warn_unsupported_provider_options,
};
pub(crate) use tool_call::parse_tool_call_arguments_json_or_string;

pub fn select_provider_options_value(
    provider_options: Option<&ProviderOptionsEnvelope>,
    provider: &str,
) -> Result<Option<Value>> {
    provider_options_envelope::select_provider_options_value(provider_options, provider)
}

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
pub(crate) fn merge_provider_options_into_body(
    body: &mut serde_json::Map<String, Value>,
    options: Option<&Value>,
    reserved_keys: &[&str],
    feature: &str,
    warnings: &mut Vec<Warning>,
) {
    provider_options_envelope::merge_provider_options_into_body(
        body,
        options,
        reserved_keys,
        feature,
        warnings,
    )
}
