use core::fmt;

macro_rules! static_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(&'static str);

        impl $name {
            pub const fn new(id: &'static str) -> Self {
                Self(id)
            }

            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0)
            }
        }
    };
}

static_id_type!(OperationKind);
static_id_type!(ApiSurfaceId);
static_id_type!(WireProtocol);
static_id_type!(ContextCacheModeId);

impl OperationKind {
    pub const CHAT_COMPLETION: Self = Self::new("chat.completion");
    pub const RESPONSE: Self = Self::new("response");
    pub const TEXT_COMPLETION: Self = Self::new("text.completion");
    pub const EMBEDDING: Self = Self::new("embedding");
    pub const MULTIMODAL_EMBEDDING: Self = Self::new("embedding.multimodal");
    pub const IMAGE_GENERATION: Self = Self::new("image.generation");
    pub const IMAGE_EDIT: Self = Self::new("image.edit");
    pub const IMAGE_TRANSLATION: Self = Self::new("image.translation");
    pub const IMAGE_QUESTION: Self = Self::new("image.question");
    pub const VIDEO_GENERATION: Self = Self::new("video.generation");
    pub const AUDIO_SPEECH: Self = Self::new("audio.speech");
    pub const AUDIO_TRANSCRIPTION: Self = Self::new("audio.transcription");
    pub const AUDIO_TRANSLATION: Self = Self::new("audio.translation");
    pub const AUDIO_VOICE_CLONE: Self = Self::new("audio.voice_clone");
    pub const AUDIO_VOICE_DESIGN: Self = Self::new("audio.voice_design");
    pub const REALTIME_SESSION: Self = Self::new("realtime.session");
    pub const RERANK: Self = Self::new("rerank");
    pub const CLASSIFICATION_OR_EXTRACTION: Self = Self::new("classification_or_extraction");
    pub const MODERATION: Self = Self::new("moderation");
    pub const BATCH: Self = Self::new("batch");
    pub const OCR: Self = Self::new("ocr");
    pub const MODEL_LIST: Self = Self::new("model.list");
    pub const CONTEXT_CACHE: Self = Self::new("context.cache");
    pub const THREAD_RUN: Self = Self::new("thread.run");
    pub const GROUP_CHAT_COMPLETION: Self = Self::new("group.chat.completion");
    pub const CHAT_TRANSLATION: Self = Self::new("chat.translation");
    pub const MUSIC_GENERATION: Self = Self::new("music.generation");
    pub const THREE_D_GENERATION: Self = Self::new("3d.generation");
}

impl ApiSurfaceId {
    pub const OPENAI_CHAT_COMPLETIONS: Self = Self::new("chat.completion");
    pub const OPENAI_RESPONSES: Self = Self::new("responses");
    pub const OPENAI_TEXT_COMPLETIONS: Self = Self::new("completion.legacy");
    pub const OPENAI_EMBEDDINGS: Self = Self::new("embedding");
    pub const OPENAI_IMAGES_GENERATIONS: Self = Self::new("image.generation");
    pub const OPENAI_IMAGES_EDITS: Self = Self::new("image.edit");
    pub const OPENAI_VIDEOS: Self = Self::new("video.generation.async");
    pub const OPENAI_AUDIO_SPEECH: Self = Self::new("audio.speech");
    pub const OPENAI_AUDIO_TRANSCRIPTIONS: Self = Self::new("audio.transcription");
    pub const OPENAI_AUDIO_TRANSLATIONS: Self = Self::new("audio.translation");
    pub const OPENAI_MODERATIONS: Self = Self::new("moderation");
    pub const OPENAI_BATCHES: Self = Self::new("batch");
    pub const OPENAI_REALTIME: Self = Self::new("realtime.websocket");
    pub const ANTHROPIC_MESSAGES: Self = Self::new("anthropic.messages");
    pub const GOOGLE_GENERATE_CONTENT: Self = Self::new("generate.content");
    pub const GOOGLE_STREAM_GENERATE_CONTENT: Self = Self::new("generate.content.stream");
    pub const GOOGLE_BATCH_GENERATE_CONTENT: Self = Self::new("generate.content.batch");
    pub const GOOGLE_EMBED_CONTENT: Self = Self::new("embedding");
    pub const GOOGLE_BATCH_EMBED_CONTENT: Self = Self::new("embedding.batch");
    pub const GOOGLE_LIVE: Self = Self::new("realtime.websocket");
    pub const GOOGLE_PREDICT: Self = Self::new("image.generation");
    pub const GOOGLE_PREDICT_LONG_RUNNING: Self = Self::new("video.generation");
}

impl WireProtocol {
    pub const OPENAI_CHAT_COMPLETIONS: Self = Self::new("openai.chat_completions");
    pub const OPENAI_RESPONSES: Self = Self::new("openai.responses");
    pub const OPENAI_TEXT_COMPLETIONS: Self = Self::new("openai.text_completions");
    pub const OPENAI_EMBEDDINGS: Self = Self::new("openai.embeddings");
    pub const OPENAI_IMAGES: Self = Self::new("openai.images");
    pub const OPENAI_VIDEOS: Self = Self::new("openai.videos");
    pub const OPENAI_AUDIO: Self = Self::new("openai.audio");
    pub const OPENAI_MODERATIONS: Self = Self::new("openai.moderations");
    pub const OPENAI_BATCHES: Self = Self::new("openai.batches");
    pub const OPENAI_REALTIME: Self = Self::new("openai.realtime");
    pub const ANTHROPIC_MESSAGES: Self = Self::new("anthropic.messages");
    pub const GOOGLE_GENERATE_CONTENT: Self = Self::new("google.generate_content");
    pub const GOOGLE_EMBED_CONTENT: Self = Self::new("google.embed_content");
    pub const GOOGLE_LIVE: Self = Self::new("google.live");
    pub const GOOGLE_PREDICT: Self = Self::new("google.predict");
    pub const GOOGLE_PREDICT_LONG_RUNNING: Self = Self::new("google.predict_long_running");
    pub const DASHSCOPE_NATIVE: Self = Self::new("dashscope.native");
    pub const DASHSCOPE_INFERENCE_WS: Self = Self::new("dashscope.inference_ws");
    pub const DASHSCOPE_REALTIME_WS: Self = Self::new("dashscope.realtime_ws");
    pub const QIANFAN_NATIVE: Self = Self::new("qianfan.native");
    pub const ARK_NATIVE: Self = Self::new("ark.native");
    pub const HUNYUAN_NATIVE: Self = Self::new("hunyuan.native");
    pub const MINIMAX_NATIVE: Self = Self::new("minimax.native");
    pub const ZHIPU_NATIVE: Self = Self::new("zhipu.native");
}

impl ContextCacheModeId {
    pub const PASSIVE: Self = Self::new("passive");
    pub const PROMPT_CACHE_KEY: Self = Self::new("prompt_cache_key");
    pub const ANTHROPIC_COMPATIBLE: Self = Self::new("anthropic_compatible");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderId<'a>(&'a str);

impl<'a> ProviderId<'a> {
    pub const fn new(id: &'a str) -> Self {
        Self(id)
    }

    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

impl fmt::Display for ProviderId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl<'a> From<&'a str> for ProviderId<'a> {
    fn from(value: &'a str) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityKind(&'static str);

impl CapabilityKind {
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }

    pub fn parse_config_token(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "llm" => Some(Self::LLM),
            "embedding" | "embeddings" => Some(Self::EMBEDDING),
            "image.generation" | "image_generation" | "image-generation" => {
                Some(Self::IMAGE_GENERATION)
            }
            "image.edit" | "image_edit" | "image-edit" => Some(Self::IMAGE_EDIT),
            "image.translation" | "image_translation" | "image-translation" => {
                Some(Self::IMAGE_TRANSLATION)
            }
            "image.question" | "image_question" | "image-question" => Some(Self::IMAGE_QUESTION),
            "video.generation" | "video_generation" | "video-generation" => {
                Some(Self::VIDEO_GENERATION)
            }
            "audio.speech" | "audio_speech" | "audio-speech" => Some(Self::AUDIO_SPEECH),
            "audio.transcription" | "audio_transcription" | "audio-transcription" => {
                Some(Self::AUDIO_TRANSCRIPTION)
            }
            "audio.translation" | "audio_translation" | "audio-translation" => {
                Some(Self::AUDIO_TRANSLATION)
            }
            "audio.voice_clone" | "audio_voice_clone" | "audio-voice-clone" => {
                Some(Self::AUDIO_VOICE_CLONE)
            }
            "audio.voice_design" | "audio_voice_design" | "audio-voice-design" => {
                Some(Self::AUDIO_VOICE_DESIGN)
            }
            "realtime" => Some(Self::REALTIME),
            "rerank" => Some(Self::RERANK),
            "classification_or_extraction" | "classification-or-extraction" => {
                Some(Self::CLASSIFICATION_OR_EXTRACTION)
            }
            "moderation" | "moderations" => Some(Self::MODERATION),
            "batch" | "batches" => Some(Self::BATCH),
            "ocr" => Some(Self::OCR),
            "model.list" | "model_list" | "model-list" | "models" => Some(Self::MODEL_LIST),
            "context.cache" | "context_cache" | "context-cache" => Some(Self::CONTEXT_CACHE),
            "music.generation" | "music_generation" | "music-generation" => {
                Some(Self::MUSIC_GENERATION)
            }
            "3d.generation" | "3d_generation" | "3d-generation" => Some(Self::THREE_D_GENERATION),
            _ => None,
        }
    }

    pub const LLM: Self = Self::new("llm");
    pub const EMBEDDING: Self = Self::new("embedding");
    pub const IMAGE_GENERATION: Self = Self::new("image.generation");
    pub const IMAGE_EDIT: Self = Self::new("image.edit");
    pub const IMAGE_TRANSLATION: Self = Self::new("image.translation");
    pub const IMAGE_QUESTION: Self = Self::new("image.question");
    pub const VIDEO_GENERATION: Self = Self::new("video.generation");
    pub const AUDIO_SPEECH: Self = Self::new("audio.speech");
    pub const AUDIO_TRANSCRIPTION: Self = Self::new("audio.transcription");
    pub const AUDIO_TRANSLATION: Self = Self::new("audio.translation");
    pub const AUDIO_VOICE_CLONE: Self = Self::new("audio.voice_clone");
    pub const AUDIO_VOICE_DESIGN: Self = Self::new("audio.voice_design");
    pub const REALTIME: Self = Self::new("realtime");
    pub const RERANK: Self = Self::new("rerank");
    pub const CLASSIFICATION_OR_EXTRACTION: Self = Self::new("classification_or_extraction");
    pub const MODERATION: Self = Self::new("moderation");
    pub const BATCH: Self = Self::new("batch");
    pub const OCR: Self = Self::new("ocr");
    pub const MODEL_LIST: Self = Self::new("model.list");
    pub const CONTEXT_CACHE: Self = Self::new("context.cache");
    pub const MUSIC_GENERATION: Self = Self::new("music.generation");
    pub const THREE_D_GENERATION: Self = Self::new("3d.generation");
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

pub fn capability_for_operation(operation: OperationKind) -> Option<CapabilityKind> {
    match operation {
        OperationKind::CHAT_COMPLETION
        | OperationKind::RESPONSE
        | OperationKind::TEXT_COMPLETION
        | OperationKind::CLASSIFICATION_OR_EXTRACTION
        | OperationKind::THREAD_RUN
        | OperationKind::GROUP_CHAT_COMPLETION
        | OperationKind::CHAT_TRANSLATION => Some(CapabilityKind::LLM),
        OperationKind::EMBEDDING | OperationKind::MULTIMODAL_EMBEDDING => {
            Some(CapabilityKind::EMBEDDING)
        }
        OperationKind::IMAGE_GENERATION => Some(CapabilityKind::IMAGE_GENERATION),
        OperationKind::IMAGE_EDIT => Some(CapabilityKind::IMAGE_EDIT),
        OperationKind::IMAGE_TRANSLATION => Some(CapabilityKind::IMAGE_TRANSLATION),
        OperationKind::IMAGE_QUESTION => Some(CapabilityKind::IMAGE_QUESTION),
        OperationKind::VIDEO_GENERATION => Some(CapabilityKind::VIDEO_GENERATION),
        OperationKind::AUDIO_SPEECH => Some(CapabilityKind::AUDIO_SPEECH),
        OperationKind::AUDIO_TRANSCRIPTION => Some(CapabilityKind::AUDIO_TRANSCRIPTION),
        OperationKind::AUDIO_TRANSLATION => Some(CapabilityKind::AUDIO_TRANSLATION),
        OperationKind::AUDIO_VOICE_CLONE => Some(CapabilityKind::AUDIO_VOICE_CLONE),
        OperationKind::AUDIO_VOICE_DESIGN => Some(CapabilityKind::AUDIO_VOICE_DESIGN),
        OperationKind::REALTIME_SESSION => Some(CapabilityKind::REALTIME),
        OperationKind::RERANK => Some(CapabilityKind::RERANK),
        OperationKind::MODERATION => Some(CapabilityKind::MODERATION),
        OperationKind::BATCH => Some(CapabilityKind::BATCH),
        OperationKind::OCR => Some(CapabilityKind::OCR),
        OperationKind::MODEL_LIST => Some(CapabilityKind::MODEL_LIST),
        OperationKind::CONTEXT_CACHE => Some(CapabilityKind::CONTEXT_CACHE),
        OperationKind::MUSIC_GENERATION => Some(CapabilityKind::MUSIC_GENERATION),
        OperationKind::THREE_D_GENERATION => Some(CapabilityKind::THREE_D_GENERATION),
        _ => None,
    }
}

const LLM_INVOCATION_OPERATIONS: &[OperationKind] = &[
    OperationKind::CHAT_COMPLETION,
    OperationKind::RESPONSE,
    OperationKind::TEXT_COMPLETION,
];
const EMBEDDING_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::EMBEDDING];
const MODERATION_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::MODERATION];
const IMAGE_GENERATION_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::IMAGE_GENERATION];
const IMAGE_EDIT_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::IMAGE_EDIT];
const VIDEO_GENERATION_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::VIDEO_GENERATION];
const REALTIME_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::REALTIME_SESSION];
const AUDIO_TRANSCRIPTION_INVOCATION_OPERATIONS: &[OperationKind] =
    &[OperationKind::AUDIO_TRANSCRIPTION];
const AUDIO_SPEECH_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::AUDIO_SPEECH];
const BATCH_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::BATCH];
const RERANK_INVOCATION_OPERATIONS: &[OperationKind] = &[OperationKind::RERANK];

/// CONTRACT-CAPABILITY-INVOCATION-OPS: ordered generic invocation operations
/// probed by runtime builders for a capability adapter.
///
/// This is intentionally not the exhaustive inverse of `capability_for_operation`.
/// It only describes the stable invocation surfaces that the generic runtime
/// builders can assemble today.
pub fn invocation_operations_for_capability(
    capability: CapabilityKind,
) -> &'static [OperationKind] {
    if capability == CapabilityKind::LLM {
        LLM_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::EMBEDDING {
        EMBEDDING_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::MODERATION {
        MODERATION_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::IMAGE_GENERATION {
        IMAGE_GENERATION_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::IMAGE_EDIT {
        IMAGE_EDIT_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::VIDEO_GENERATION {
        VIDEO_GENERATION_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::REALTIME {
        REALTIME_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::AUDIO_TRANSCRIPTION {
        AUDIO_TRANSCRIPTION_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::AUDIO_SPEECH {
        AUDIO_SPEECH_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::BATCH {
        BATCH_INVOCATION_OPERATIONS
    } else if capability == CapabilityKind::RERANK {
        RERANK_INVOCATION_OPERATIONS
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::{CapabilityKind, OperationKind, invocation_operations_for_capability};

    #[test]
    fn llm_invocation_operations_cover_generic_text_surfaces() {
        assert_eq!(
            invocation_operations_for_capability(CapabilityKind::LLM),
            &[
                OperationKind::CHAT_COMPLETION,
                OperationKind::RESPONSE,
                OperationKind::TEXT_COMPLETION,
            ]
        );
    }

    #[test]
    fn batch_and_rerank_invocation_operations_are_single_surface() {
        assert_eq!(
            invocation_operations_for_capability(CapabilityKind::BATCH),
            &[OperationKind::BATCH]
        );
        assert_eq!(
            invocation_operations_for_capability(CapabilityKind::RERANK),
            &[OperationKind::RERANK]
        );
    }
}
