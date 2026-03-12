#[cfg(feature = "provider-google")]
use ditto_llm::config::ProviderApi;
use ditto_llm::config::ProviderConfig;
use ditto_llm::contracts::{CapabilityKind, OperationKind, RuntimeRouteRequest};
use ditto_llm::runtime::{
    RuntimeBaseUrlSelectionSource, RuntimeModelSelectionSource, RuntimeProviderSelectionSource,
};

#[test]
fn runtime_explain_reports_provider_config_sources() {
    let explain = ditto_llm::runtime::explain_builtin_runtime_route(
        RuntimeRouteRequest::new("openai-compatible", None, OperationKind::CHAT_COMPLETION)
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
    .expect("runtime explain should resolve generic openai-compatible route");

    assert_eq!(explain.provider_hint, "openai-compatible");
    assert_eq!(explain.resolved_provider, "openai-compatible");
    assert_eq!(
        explain.provider_source,
        RuntimeProviderSelectionSource::RequestProvider
    );
    assert_eq!(
        explain.model_source,
        RuntimeModelSelectionSource::ProviderDefaultModel
    );
    assert_eq!(
        explain.base_url_source,
        RuntimeBaseUrlSelectionSource::ProviderConfig
    );
    assert_eq!(
        explain.route.url,
        "https://proxy.example/v1/chat/completions"
    );
    assert!(
        explain
            .capability_resolution
            .effective_capabilities
            .contains(&CapabilityKind::LLM.as_str())
    );

    let json = serde_json::to_value(&explain).expect("runtime explain should serialize");
    assert!(json.get("route").is_some());
}

#[cfg(feature = "provider-google")]
#[test]
fn runtime_explain_reports_upstream_api_fallback_for_yunwu() {
    let provider_config = ProviderConfig {
        base_url: Some("https://yunwu.ai/v1beta".to_string()),
        default_model: Some("gemini-3.1-pro".to_string()),
        upstream_api: Some(ProviderApi::GeminiGenerateContent),
        ..ProviderConfig::default()
    };

    let explain = ditto_llm::runtime::explain_builtin_runtime_route(
        RuntimeRouteRequest::new("yunwu", None, OperationKind::CHAT_COMPLETION)
            .with_runtime_hints(provider_config.runtime_hints())
            .with_required_capability(CapabilityKind::LLM),
    )
    .expect("custom yunwu gemini route should explain via google fallback");

    assert_eq!(explain.provider_hint, "yunwu");
    assert_eq!(explain.resolved_provider, "google");
    assert_eq!(
        explain.provider_source,
        RuntimeProviderSelectionSource::UpstreamApiFallback
    );
    assert_eq!(
        explain.model_source,
        RuntimeModelSelectionSource::ProviderDefaultModel
    );
    assert_eq!(
        explain.base_url_source,
        RuntimeBaseUrlSelectionSource::ProviderConfig
    );
    assert!(explain.route.url.starts_with("https://yunwu.ai/v1beta/"));
    assert!(explain.route.url.ends_with(":generateContent"));
}
