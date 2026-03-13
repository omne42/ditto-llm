use ditto_core::config::ProviderConfig;
use ditto_core::contracts::{OperationKind, RuntimeRouteRequest};
use ditto_core::runtime::resolve_builtin_runtime_route;
use ditto_core::runtime_registry::builtin_runtime_registry_catalog;

#[test]
fn catalog_summary_exposes_default_openai_compatible_llm_surface() {
    let summary = builtin_runtime_registry_catalog()
        .provider_capability_summaries()
        .into_iter()
        .find(|summary| summary.provider == "openai-compatible")
        .expect("openai-compatible summary should exist");

    assert!(
        summary
            .capabilities
            .iter()
            .any(|capability| capability.as_str() == "llm")
    );
}

#[test]
fn catalog_resolver_resolves_generic_openai_compatible_chat_path() {
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
        ),
    )
    .expect("generic openai-compatible route should resolve");

    assert_eq!(route.url, "https://proxy.example/v1/chat/completions");
}
