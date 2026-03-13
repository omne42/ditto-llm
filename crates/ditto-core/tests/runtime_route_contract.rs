use ditto_core::config::ProviderConfig;
use ditto_core::contracts::{CapabilityKind, OperationKind, RuntimeRouteRequest};
use ditto_core::foundation::error::DittoError;
use ditto_core::runtime::resolve_builtin_runtime_route;

#[test]
fn runtime_route_rejects_non_llm_capability_when_embeddings_are_disabled() {
    let result = resolve_builtin_runtime_route(
        RuntimeRouteRequest::new(
            "openai-compatible",
            Some("text-embedding-3-small"),
            OperationKind::EMBEDDING,
        )
        .with_runtime_hints(
            ProviderConfig {
                base_url: Some("https://proxy.example/v1".to_string()),
                default_model: Some("text-embedding-3-small".to_string()),
                ..ProviderConfig::default()
            }
            .runtime_hints(),
        )
        .with_required_capability(CapabilityKind::EMBEDDING),
    );

    if cfg!(feature = "embeddings") {
        assert!(
            result.is_ok(),
            "embedding-enabled builds should expose embedding routes"
        );
    } else {
        let err = result.expect_err("default core should not expose embedding routes");
        assert!(matches!(
            err,
            DittoError::ProviderResolution(
                ditto_core::foundation::error::ProviderResolutionError::RuntimeRouteCapabilityUnsupported { .. }
            )
        ));
    }
}

#[test]
fn runtime_route_resolves_generic_openai_compatible_llm_path() {
    let route = resolve_builtin_runtime_route(
        RuntimeRouteRequest::new(
            "openai-compatible",
            Some("gpt-4o-mini"),
            OperationKind::CHAT_COMPLETION,
        )
        .with_runtime_hints(
            ProviderConfig {
                base_url: Some("https://proxy.example/v1".to_string()),
                default_model: Some("gpt-4o-mini".to_string()),
                ..ProviderConfig::default()
            }
            .runtime_hints(),
        )
        .with_required_capability(CapabilityKind::LLM),
    )
    .expect("generic openai-compatible llm route should resolve");

    assert_eq!(route.base_url, "https://proxy.example/v1");
    assert_eq!(route.url, "https://proxy.example/v1/chat/completions");
}
