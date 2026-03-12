use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{ImageSource, Usage, Warning};
use crate::provider_options::ProviderOptionsEnvelope;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageResponseFormat {
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "b64_json")]
    Base64Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ImageResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageGenerationResponse {
    #[serde(default)]
    pub images: Vec<ImageSource>,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageEditUpload {
    pub data: Vec<u8>,
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEditRequest {
    pub prompt: String,
    pub images: Vec<ImageEditUpload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<ImageEditUpload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ImageResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<ProviderOptionsEnvelope>,
}

pub type ImageEditResponse = ImageGenerationResponse;
pub type VideoReferenceUpload = ImageEditUpload;
