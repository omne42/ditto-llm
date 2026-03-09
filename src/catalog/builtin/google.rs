use crate::catalog::generated::providers::GOOGLE_PLUGIN;
use crate::catalog::{CapabilityKind, CapabilityStatusDescriptor, ProviderPluginDescriptor};

const GOOGLE_RUNTIME_CAPABILITY_STATUSES: &[CapabilityStatusDescriptor] = &[
    CapabilityStatusDescriptor::implemented(CapabilityKind::LLM),
    #[cfg(feature = "embeddings")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::EMBEDDING),
    #[cfg(not(feature = "embeddings"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::EMBEDDING),
    #[cfg(feature = "images")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::IMAGE_GENERATION),
    #[cfg(not(feature = "images"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::IMAGE_GENERATION),
    #[cfg(feature = "realtime")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::REALTIME),
    #[cfg(not(feature = "realtime"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::REALTIME),
    #[cfg(feature = "videos")]
    CapabilityStatusDescriptor::implemented(CapabilityKind::VIDEO_GENERATION),
    #[cfg(not(feature = "videos"))]
    CapabilityStatusDescriptor::planned(CapabilityKind::VIDEO_GENERATION),
];

pub const BUILTIN_GOOGLE_PLUGIN: ProviderPluginDescriptor = ProviderPluginDescriptor {
    capability_statuses: GOOGLE_RUNTIME_CAPABILITY_STATUSES,
    ..GOOGLE_PLUGIN
};
