use std::any::TypeId;

#[test]
fn layered_namespaces_expose_core_and_capabilities() {
    let _ = TypeId::of::<ditto_llm::core::DittoError>();
    let _ = TypeId::of::<ditto_llm::core::CollectedStream>();
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
