use std::any::TypeId;

#[test]
fn direct_l0_namespaces_expose_low_level_owners() {
    let _ = TypeId::of::<ditto_core::foundation::error::DittoError>();
    let _ = TypeId::of::<ditto_core::llm_core::stream::CollectedStream>();
    let _ = TypeId::of::<dyn ditto_core::llm_core::model::LanguageModel>();
    let _ = TypeId::of::<ditto_core::contracts::RuntimeRoute>();
    let _ = TypeId::of::<ditto_core::contracts::FinishReason>();
    let _ = TypeId::of::<ditto_core::contracts::Usage>();
    let _ = TypeId::of::<ditto_core::contracts::Warning>();
    let _ = TypeId::of::<ditto_core::contracts::GenerateRequest>();
    let _ = TypeId::of::<ditto_core::contracts::GenerateResponse>();
    let _ = TypeId::of::<ditto_core::contracts::StreamChunk>();
    let _ = TypeId::of::<ditto_core::provider_options::ProviderOptionsEnvelope>();
    let _ = TypeId::of::<ditto_core::runtime_registry::RuntimeRegistrySnapshot>();
    let _ = ditto_core::runtime::build_language_model;
    let _ = ditto_core::runtime::build_context_cache_model;
}

#[test]
fn northbound_capability_facades_remain_available() {
    let _ = TypeId::of::<ditto_core::capabilities::text::GenerateTextResponse>();
    let _ = TypeId::of::<ditto_core::capabilities::file::FileObject>();
    let _ = TypeId::of::<ditto_core::capabilities::context_cache::ContextCacheProfile>();
}

#[cfg(feature = "gateway")]
#[test]
fn gateway_layered_facades_expose_transport_domain_and_adapters() {
    let _ = TypeId::of::<ditto_server::gateway::transport::http::GatewayHttpState>();
    let _ = TypeId::of::<ditto_server::gateway::domain::GatewayRequest>();
    let _ = TypeId::of::<ditto_server::gateway::adapters::backend::ProxyBackend>();
    let _ = TypeId::of::<ditto_server::gateway::adapters::state::GatewayStateFile>();
}
