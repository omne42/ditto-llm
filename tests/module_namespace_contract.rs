use std::any::TypeId;

#[test]
fn direct_l0_namespaces_expose_low_level_owners() {
    let _ = TypeId::of::<ditto_llm::foundation::error::DittoError>();
    let _ = TypeId::of::<ditto_llm::llm_core::stream::CollectedStream>();
    let _ = TypeId::of::<dyn ditto_llm::llm_core::model::LanguageModel>();
    let _ = TypeId::of::<ditto_llm::contracts::RuntimeRoute>();
    let _ = TypeId::of::<ditto_llm::contracts::FinishReason>();
    let _ = TypeId::of::<ditto_llm::contracts::Usage>();
    let _ = TypeId::of::<ditto_llm::contracts::Warning>();
    let _ = TypeId::of::<ditto_llm::contracts::GenerateRequest>();
    let _ = TypeId::of::<ditto_llm::contracts::GenerateResponse>();
    let _ = TypeId::of::<ditto_llm::contracts::StreamChunk>();
    let _ = TypeId::of::<ditto_llm::provider_options::ProviderOptionsEnvelope>();
    let _ = TypeId::of::<ditto_llm::runtime_registry::RuntimeRegistrySnapshot>();
    let _ = ditto_llm::runtime::build_language_model;
    let _ = ditto_llm::runtime::build_context_cache_model;
}

#[test]
fn northbound_capability_facades_remain_available() {
    let _ = TypeId::of::<ditto_llm::capabilities::text::GenerateTextResponse>();
    let _ = TypeId::of::<ditto_llm::capabilities::file::FileObject>();
    let _ = TypeId::of::<ditto_llm::capabilities::context_cache::ContextCacheProfile>();
}

#[cfg(feature = "gateway")]
#[test]
fn gateway_layered_facades_expose_transport_domain_and_adapters() {
    let _ = TypeId::of::<ditto_llm::gateway::transport::http::GatewayHttpState>();
    let _ = TypeId::of::<ditto_llm::gateway::domain::GatewayRequest>();
    let _ = TypeId::of::<ditto_llm::gateway::adapters::backend::ProxyBackend>();
    let _ = TypeId::of::<ditto_llm::gateway::adapters::state::GatewayStateFile>();
}
