use std::any::TypeId;

#[test]
fn direct_l0_namespaces_expose_public_facades() {
    let _ = TypeId::of::<ditto_core::error::DittoError>();
    let _ = TypeId::of::<dyn ditto_core::llm_core::model::LanguageModel>();
    let _ = TypeId::of::<ditto_core::contracts::RuntimeRoute>();
    let _ = TypeId::of::<ditto_core::contracts::FinishReason>();
    let _ = TypeId::of::<ditto_core::contracts::Usage>();
    let _ = TypeId::of::<ditto_core::contracts::Warning>();
    let _ = TypeId::of::<ditto_core::contracts::GenerateRequest>();
    let _ = TypeId::of::<ditto_core::contracts::GenerateResponse>();
    let _ = TypeId::of::<ditto_core::contracts::StreamChunk>();
    let _ = TypeId::of::<ditto_core::config::ProviderConfig>();
    let _ = TypeId::of::<ditto_core::provider_options::ProviderOptionsEnvelope>();
}

#[test]
fn northbound_capability_facades_remain_available() {
    let _ = TypeId::of::<ditto_core::capabilities::text::GenerateTextResponse>();
    let _ = TypeId::of::<ditto_core::capabilities::file::FileObject>();
    let _ = TypeId::of::<ditto_core::capabilities::context_cache::ContextCacheProfile>();
}

#[cfg(feature = "gateway")]
#[test]
fn gateway_root_exports_stable_public_facades() {
    let _ = TypeId::of::<ditto_server::gateway::Gateway>();
    let _ = TypeId::of::<ditto_server::gateway::GatewayConfig>();
    let _ = TypeId::of::<ditto_server::gateway::BackendConfig>();
    let _ = TypeId::of::<ditto_server::gateway::VirtualKeyConfig>();
    let _ = TypeId::of::<ditto_server::gateway::RouteBackend>();
    let _ = TypeId::of::<ditto_server::gateway::GatewayRequest>();
    let _ = TypeId::of::<ditto_server::gateway::GatewayHttpState>();
    let _ = TypeId::of::<ditto_server::gateway::ProxyBackend>();
    let _ = TypeId::of::<ditto_server::gateway::GatewayStateFile>();
}
