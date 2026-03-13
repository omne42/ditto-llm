use axum::http::Method;

use crate::contracts::{CapabilityKind, OperationKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationEndpointKind {
    ChatCompletions,
    Completions,
    ResponsesCreate,
    ResponsesCompact,
    ResponsesInputTokens,
    ResponsesRetrieve,
    ResponsesInputItems,
    Embeddings,
    Moderations,
    ImagesGenerations,
    ImagesEdits,
    AudioTranscriptions,
    AudioTranslations,
    VideosRoot,
    VideoRetrieve,
    VideoContent,
    VideoRemix,
    AudioSpeech,
    BatchesRoot,
    BatchRetrieve,
    BatchCancel,
    Rerank,
    ModelsList,
    ModelsRetrieve,
    FilesRoot,
    FilesRetrieve,
    FilesContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationEndpointRequirement {
    None,
    RuntimeCapability(&'static [CapabilityKind]),
    FilesApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationEndpointDescriptor {
    pub kind: TranslationEndpointKind,
    pub runtime_operation: Option<OperationKind>,
    pub requirement: TranslationEndpointRequirement,
}

const LLM_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::LLM];
const EMBEDDING_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::EMBEDDING];
const MODERATION_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::MODERATION];
const IMAGE_GENERATION_RUNTIME_CAPABILITIES: &[CapabilityKind] =
    &[CapabilityKind::IMAGE_GENERATION];
const IMAGE_EDIT_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::IMAGE_EDIT];
const AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES: &[CapabilityKind] =
    &[CapabilityKind::AUDIO_TRANSCRIPTION];
const VIDEO_GENERATION_RUNTIME_CAPABILITIES: &[CapabilityKind] =
    &[CapabilityKind::VIDEO_GENERATION];
const AUDIO_SPEECH_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::AUDIO_SPEECH];
const RERANK_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::RERANK];
const BATCH_RUNTIME_CAPABILITIES: &[CapabilityKind] = &[CapabilityKind::BATCH];

fn path_without_query(path_and_query: &str) -> &str {
    path_and_query
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_and_query)
}

fn singleton_path_matches(path_and_query: &str, expected: &str) -> bool {
    let path = path_without_query(path_and_query);
    path == expected || path == format!("{expected}/")
}

pub fn is_chat_completions_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/chat/completions")
}

pub fn is_completions_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/completions")
}

pub fn is_models_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/models")
}

pub fn models_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/models/")?;
    if rest.trim().is_empty() {
        return None;
    }
    Some(rest.to_string())
}

pub fn is_responses_create_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/responses")
}

pub fn is_responses_compact_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/responses/compact")
}

pub fn is_responses_input_tokens_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/responses/input_tokens")
}

pub fn is_embeddings_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/embeddings")
}

pub fn is_moderations_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/moderations")
}

pub fn is_images_generations_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/images/generations")
}

pub fn is_images_edits_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/images/edits")
}

pub fn is_audio_transcriptions_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/audio/transcriptions")
}

pub fn is_audio_translations_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/audio/translations")
}

pub fn is_videos_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/videos")
}

pub fn is_audio_speech_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/audio/speech")
}

pub fn is_batches_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/batches")
}

pub fn is_rerank_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/rerank")
}

pub fn is_files_path(path_and_query: &str) -> bool {
    singleton_path_matches(path_and_query, "/v1/files")
}

pub fn translation_endpoint_descriptor(
    method: &Method,
    path_and_query: &str,
) -> Option<TranslationEndpointDescriptor> {
    if *method == Method::POST {
        if is_chat_completions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ChatCompletions,
                runtime_operation: Some(OperationKind::CHAT_COMPLETION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_completions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Completions,
                runtime_operation: Some(OperationKind::TEXT_COMPLETION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_create_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesCreate,
                runtime_operation: Some(OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_compact_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesCompact,
                runtime_operation: Some(OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_responses_input_tokens_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesInputTokens,
                runtime_operation: Some(OperationKind::RESPONSE),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    LLM_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_embeddings_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Embeddings,
                runtime_operation: Some(OperationKind::EMBEDDING),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    EMBEDDING_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_moderations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Moderations,
                runtime_operation: Some(OperationKind::MODERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    MODERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_images_generations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ImagesGenerations,
                runtime_operation: Some(OperationKind::IMAGE_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    IMAGE_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_images_edits_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ImagesEdits,
                runtime_operation: Some(OperationKind::IMAGE_EDIT),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    IMAGE_EDIT_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_transcriptions_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioTranscriptions,
                runtime_operation: Some(OperationKind::AUDIO_TRANSCRIPTION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_translations_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioTranslations,
                runtime_operation: Some(OperationKind::AUDIO_TRANSCRIPTION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_TRANSCRIPTION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_videos_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideosRoot,
                runtime_operation: Some(OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_remix_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoRemix,
                runtime_operation: Some(OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_audio_speech_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::AudioSpeech,
                runtime_operation: Some(OperationKind::AUDIO_SPEECH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    AUDIO_SPEECH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_rerank_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::Rerank,
                runtime_operation: Some(OperationKind::RERANK),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    RERANK_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_batches_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchesRoot,
                runtime_operation: Some(OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if batches_cancel_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchCancel,
                runtime_operation: Some(OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_files_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRoot,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
    } else if *method == Method::GET {
        if is_batches_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchesRoot,
                runtime_operation: Some(OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if responses_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if responses_input_items_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ResponsesInputItems,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if batches_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::BatchRetrieve,
                runtime_operation: Some(OperationKind::BATCH),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    BATCH_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_videos_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideosRoot,
                runtime_operation: Some(OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoRetrieve,
                runtime_operation: Some(OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if videos_content_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::VideoContent,
                runtime_operation: Some(OperationKind::VIDEO_GENERATION),
                requirement: TranslationEndpointRequirement::RuntimeCapability(
                    VIDEO_GENERATION_RUNTIME_CAPABILITIES,
                ),
            });
        }
        if is_models_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ModelsList,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if models_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::ModelsRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::None,
            });
        }
        if is_files_path(path_and_query) {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRoot,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
        if files_retrieve_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesRetrieve,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
        if files_content_id(path_and_query).is_some() {
            return Some(TranslationEndpointDescriptor {
                kind: TranslationEndpointKind::FilesContent,
                runtime_operation: None,
                requirement: TranslationEndpointRequirement::FilesApi,
            });
        }
    } else if *method == Method::DELETE && videos_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::VideoRetrieve,
            runtime_operation: Some(OperationKind::VIDEO_GENERATION),
            requirement: TranslationEndpointRequirement::RuntimeCapability(
                VIDEO_GENERATION_RUNTIME_CAPABILITIES,
            ),
        });
    } else if *method == Method::DELETE && responses_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::ResponsesRetrieve,
            runtime_operation: None,
            requirement: TranslationEndpointRequirement::None,
        });
    } else if *method == Method::DELETE && files_retrieve_id(path_and_query).is_some() {
        return Some(TranslationEndpointDescriptor {
            kind: TranslationEndpointKind::FilesRetrieve,
            runtime_operation: None,
            requirement: TranslationEndpointRequirement::FilesApi,
        });
    }

    None
}

fn responses_subresource_id(path_and_query: &str, suffix: Option<&str>) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/responses/")?;
    if rest.trim().is_empty() || rest == "compact" || rest == "input_tokens" {
        return None;
    }

    match suffix {
        Some(suffix) => {
            let (response_id, found_suffix) = rest.split_once('/')?;
            if response_id.trim().is_empty() || found_suffix != suffix {
                return None;
            }
            Some(response_id.to_string())
        }
        None => {
            if rest.contains('/') {
                return None;
            }
            Some(rest.to_string())
        }
    }
}

pub fn responses_retrieve_id(path_and_query: &str) -> Option<String> {
    responses_subresource_id(path_and_query, None)
}

pub fn responses_input_items_id(path_and_query: &str) -> Option<String> {
    responses_subresource_id(path_and_query, Some("input_items"))
}

pub fn batches_cancel_id(path_and_query: &str) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    let (batch_id, suffix) = rest.split_once('/')?;
    if batch_id.trim().is_empty() {
        return None;
    }
    if suffix == "cancel" {
        return Some(batch_id.to_string());
    }
    None
}

fn videos_subresource_id(path_and_query: &str, suffix: Option<&str>) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/videos/")?;
    if rest.trim().is_empty() {
        return None;
    }

    match suffix {
        Some(suffix) => {
            let (video_id, found_suffix) = rest.split_once('/')?;
            if video_id.trim().is_empty() || found_suffix != suffix {
                return None;
            }
            Some(video_id.to_string())
        }
        None => {
            if rest.contains('/') {
                return None;
            }
            Some(rest.to_string())
        }
    }
}

pub fn videos_retrieve_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, None)
}

pub fn videos_content_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, Some("content"))
}

pub fn videos_remix_id(path_and_query: &str) -> Option<String> {
    videos_subresource_id(path_and_query, Some("remix"))
}

pub fn batches_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/batches/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

pub fn files_retrieve_id(path_and_query: &str) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    if rest.trim().is_empty() {
        return None;
    }
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

pub fn files_content_id(path_and_query: &str) -> Option<String> {
    let path = path_without_query(path_and_query).trim_end_matches('/');
    let rest = path.strip_prefix("/v1/files/")?;
    let (file_id, suffix) = rest.split_once('/')?;
    if suffix != "content" {
        return None;
    }
    let file_id = file_id.trim();
    if file_id.is_empty() {
        return None;
    }
    Some(file_id.to_string())
}
