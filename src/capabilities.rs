//! Capability facade.
//!
//! This namespace groups the public capability traits and DTOs by modality,
//! without exposing provider-specific implementation details.

pub mod audio {
    pub use crate::audio::{
        AudioTranscriptionModel, AudioTranslationModel, AudioTranslationRequest,
        AudioTranslationResponse, SpeechModel,
    };
    pub use crate::types::{
        AudioTranscriptionRequest, AudioTranscriptionResponse, SpeechRequest, SpeechResponse,
        SpeechResponseFormat, TranscriptionResponseFormat,
    };
}

pub mod batch {
    pub use crate::batch::BatchClient;
    pub use crate::types::{
        Batch, BatchCreateRequest, BatchListResponse, BatchRequestCounts, BatchResponse,
        BatchStatus,
    };
}

pub mod context_cache {
    pub use crate::context_cache::{ContextCacheMode, ContextCacheModel, ContextCacheProfile};
}

pub mod embedding {
    pub use crate::embedding::{EmbeddingModel, EmbeddingModelExt};
}

pub mod file {
    pub use crate::file::{
        FileClient, FileContent, FileDeleteResponse, FileObject, FileUploadRequest,
    };
}

pub mod image {
    pub use crate::image::ImageGenerationModel;
    pub use crate::types::{
        ImageGenerationRequest, ImageGenerationResponse, ImageResponseFormat, ImageSource,
    };
}

pub mod image_edit {
    pub use crate::image_edit::ImageEditModel;
    pub use crate::types::{ImageEditRequest, ImageEditResponse, ImageEditUpload, ImageSource};
}

pub mod moderation {
    pub use crate::moderation::ModerationModel;
    pub use crate::types::{
        ModerationInput, ModerationRequest, ModerationResponse, ModerationResult,
    };
}

pub mod object {
    pub use crate::object::{
        GenerateObjectResponse, LanguageModelObjectExt, ObjectOptions, ObjectOutput,
        ObjectStrategy, StreamObjectFinal, StreamObjectHandle, StreamObjectResult,
    };
}

pub mod realtime {
    pub use crate::realtime::{
        RealtimeSessionConnection, RealtimeSessionModel, RealtimeSessionRequest,
    };
}

pub mod rerank {
    pub use crate::rerank::RerankModel;
    pub use crate::types::{RerankDocument, RerankRequest, RerankResponse, RerankResult};
}

pub mod text {
    pub use crate::text::{
        GenerateTextResponse, LanguageModelTextExt, StreamTextFinal, StreamTextHandle,
        StreamTextResult,
    };
    pub use crate::types::{
        ContentPart, FinishReason, GenerateRequest, GenerateResponse, JsonSchemaFormat, Message,
        ProviderOptions, ReasoningEffort, ReasoningSummary, ResponseFormat, Role, StreamChunk,
        Tool, ToolChoice, Usage, Warning,
    };
}

pub mod video {
    pub use crate::types::{
        VideoContentVariant, VideoDeleteResponse, VideoGenerationError, VideoGenerationRequest,
        VideoGenerationResponse, VideoGenerationStatus, VideoListOrder, VideoListRequest,
        VideoListResponse, VideoReferenceUpload, VideoRemixRequest,
    };
    pub use crate::video::VideoGenerationModel;
}

pub use audio::{
    AudioTranscriptionModel, AudioTranslationModel, AudioTranslationRequest,
    AudioTranslationResponse, SpeechModel,
};
pub use batch::BatchClient;
pub use context_cache::{ContextCacheMode, ContextCacheModel, ContextCacheProfile};
pub use embedding::{EmbeddingModel, EmbeddingModelExt};
pub use file::{FileClient, FileContent, FileDeleteResponse, FileObject, FileUploadRequest};
pub use image::ImageGenerationModel;
pub use image_edit::ImageEditModel;
pub use moderation::ModerationModel;
pub use object::{
    GenerateObjectResponse, LanguageModelObjectExt, ObjectOptions, ObjectOutput, ObjectStrategy,
    StreamObjectFinal, StreamObjectHandle, StreamObjectResult,
};
pub use realtime::{RealtimeSessionConnection, RealtimeSessionModel, RealtimeSessionRequest};
pub use rerank::RerankModel;
pub use text::{
    GenerateTextResponse, LanguageModelTextExt, StreamTextFinal, StreamTextHandle, StreamTextResult,
};
pub use video::VideoGenerationModel;
