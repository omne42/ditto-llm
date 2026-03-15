use crate::catalog::generated::providers::GOOGLE_PLUGIN;
use crate::catalog::{CapabilityKind, CapabilityStatusDescriptor, ProviderPluginDescriptor};

const GOOGLE_RUNTIME_CAPABILITY_STATUSES: &[CapabilityStatusDescriptor] = &[
    CapabilityStatusDescriptor::implemented(CapabilityKind::LLM),
    #[cfg(feature = "cap-embedding")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::EMBEDDING),
    #[cfg(not(feature = "cap-embedding"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::EMBEDDING),
    #[cfg(any(feature = "cap-image-generation", feature = "cap-image-edit"))]
    CapabilityStatusDescriptor::implemented(CapabilityKind::IMAGE_GENERATION),
    #[cfg(not(any(feature = "cap-image-generation", feature = "cap-image-edit")))]
    CapabilityStatusDescriptor::planned(CapabilityKind::IMAGE_GENERATION),
    #[cfg(feature = "cap-realtime")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::REALTIME),
    #[cfg(not(feature = "cap-realtime"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::REALTIME),
    #[cfg(feature = "cap-video-generation")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::VIDEO_GENERATION),
    #[cfg(not(feature = "cap-video-generation"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::VIDEO_GENERATION),
];

pub const BUILTIN_GOOGLE_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
    capability_statuses: GOOGLE_RUNTIME_CAPABILITY_STATUSES,
    ..GOOGLE_PLUGIN
};
