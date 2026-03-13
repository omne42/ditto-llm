//! Capability facade.
//!
//! This namespace groups the public capability traits and DTOs by modality,
//! without exposing provider-specific implementation details.
// CAPABILITIES-FACADE-NOT-L0-OWNER: this is a northbound convenience surface
// over runtime/model adapters, not a separate L0 ownership boundary.

pub mod audio;
pub mod batch;
pub mod context_cache;
pub mod embedding;
pub mod file;
pub mod image;
pub mod image_edit;
pub mod moderation;
pub mod object;
pub mod realtime;
pub mod rerank;
pub mod text;
pub mod video;

pub use audio::{AudioTranscriptionModel, AudioTranslationModel, SpeechModel};
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
