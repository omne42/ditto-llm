#![cfg(feature = "provider-deepseek")]

use std::collections::BTreeSet;

use ditto_llm::{
    AssistantToolFollowupRequirement, BehaviorSupport, CacheUsageReportingKind, CapabilityKind,
    ContextCacheModeId, DittoError, Env, OperationKind, ProviderAuth, ProviderConfig,
    ProviderProtocolFamily, ProviderResolutionError, ReasoningActivationKind, ReasoningOutputMode,
    builtin_registry,
};

fn deepseek_env() -> Env {
    Env::parse_dotenv("DEEPSEEK_API_KEY=sk-test\n")
}

fn deepseek_config(default_model: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: Some("https://api.deepseek.com".to_string()),
        default_model: Some(default_model.to_string()),
        auth: Some(ProviderAuth::ApiKeyEnv {
            keys: vec!["DEEPSEEK_API_KEY".to_string()],
        }),
        ..ProviderConfig::default()
    }
}

#[test]
fn deepseek_catalog_runtime_spec_matches_enabled_capabilities() {
    let plugin = builtin_registry()
        .plugin("deepseek")
        .expect("deepseek plugin should be available");
    let runtime_spec = plugin.runtime_spec();
    let actual = runtime_spec
        .capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(runtime_spec.protocol_family, ProviderProtocolFamily::OpenAi);
    assert_eq!(
        actual,
        BTreeSet::from([
            CapabilityKind::CONTEXT_CACHE.as_str(),
            CapabilityKind::LLM.as_str(),
        ])
    );

    let reasoning_caps = plugin.capability_resolution(Some("deepseek-reasoner"));
    assert!(reasoning_caps.effective_supports(CapabilityKind::LLM));
    assert!(reasoning_caps.effective_supports(CapabilityKind::CONTEXT_CACHE));
    assert!(
        builtin_registry()
            .resolve(
                "deepseek",
                "deepseek-reasoner",
                OperationKind::CHAT_COMPLETION
            )
            .is_some()
    );
    assert!(
        builtin_registry()
            .plugin("deepseek")
            .expect("deepseek plugin should be available")
            .capability_resolution(Some("deepseek-reasoner"))
            .effective_supports(CapabilityKind::CONTEXT_CACHE)
    );
}

#[test]
fn deepseek_catalog_exposes_beta_fim_route_and_model_behaviors() {
    let fim = builtin_registry()
        .resolve("deepseek", "deepseek-chat", OperationKind::TEXT_COMPLETION)
        .expect("deepseek-chat FIM binding should exist");
    assert_eq!(
        fim.endpoint.base_url_override.as_deref(),
        Some("https://api.deepseek.com/beta")
    );
    assert_eq!(fim.endpoint.path, "/completions");

    let chat_behavior = builtin_registry()
        .behavior("deepseek", "deepseek-chat", OperationKind::CHAT_COMPLETION)
        .expect("deepseek-chat behavior should exist");
    assert_eq!(chat_behavior.tool_calls, BehaviorSupport::Supported);
    assert_eq!(
        chat_behavior.tool_choice_required,
        BehaviorSupport::Supported
    );
    assert_eq!(
        chat_behavior.assistant_tool_followup,
        AssistantToolFollowupRequirement::None
    );
    assert_eq!(
        chat_behavior.reasoning_output,
        ReasoningOutputMode::Optional
    );
    assert_eq!(
        chat_behavior.reasoning_activation,
        ReasoningActivationKind::DeepSeekThinkingTypeEnabled
    );
    assert!(
        chat_behavior
            .context_cache_modes
            .contains(&ContextCacheModeId::PASSIVE)
    );
    assert!(chat_behavior.context_cache_default_enabled);
    assert_eq!(
        chat_behavior.cache_usage_reporting,
        CacheUsageReportingKind::DeepSeekPromptCacheHitMiss
    );

    let fim_behavior = builtin_registry()
        .behavior("deepseek", "deepseek-chat", OperationKind::TEXT_COMPLETION)
        .expect("deepseek-chat FIM behavior should exist");
    assert_eq!(fim_behavior.tool_calls, BehaviorSupport::Unsupported);
    assert_eq!(
        fim_behavior.reasoning_output,
        ReasoningOutputMode::Unsupported
    );

    let reasoner_behavior = builtin_registry()
        .behavior(
            "deepseek",
            "deepseek-reasoner",
            OperationKind::CHAT_COMPLETION,
        )
        .expect("deepseek-reasoner behavior should exist");
    assert_eq!(reasoner_behavior.tool_calls, BehaviorSupport::Supported);
    assert_eq!(
        reasoner_behavior.tool_choice_required,
        BehaviorSupport::Unsupported
    );
    assert_eq!(
        reasoner_behavior.assistant_tool_followup,
        AssistantToolFollowupRequirement::RequiresReasoningContent
    );
    assert_eq!(
        reasoner_behavior.reasoning_output,
        ReasoningOutputMode::Always
    );
    assert_eq!(
        reasoner_behavior.reasoning_activation,
        ReasoningActivationKind::AlwaysOn
    );
    assert!(
        reasoner_behavior
            .context_cache_modes
            .contains(&ContextCacheModeId::PASSIVE)
    );
    assert!(reasoner_behavior.context_cache_default_enabled);
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-deepseek",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_deepseek_llm() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_language_model(
        "deepseek",
        &deepseek_config("deepseek-reasoner"),
        &deepseek_env(),
    )
    .await?;

    assert_eq!(model.provider(), "openai-compatible");
    assert_eq!(model.model_id(), "deepseek-reasoner");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-deepseek",
    feature = "cap-embedding"
))]
#[tokio::test]
async fn gateway_builder_rejects_unimplemented_deepseek_embedding_capability() {
    let err = match ditto_llm::gateway::translation::build_embedding_model(
        "deepseek",
        &deepseek_config("deepseek-chat"),
        &deepseek_env(),
    )
    .await
    {
        Ok(_) => panic!("deepseek should reject embedding builder requests"),
        Err(err) => err,
    };

    assert!(matches!(
        err,
        DittoError::ProviderResolution(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
            ref provider,
            ref capability,
            ..
        }) if provider == "deepseek" && capability == CapabilityKind::EMBEDDING.as_str()
    ));
}

#[cfg(all(feature = "gateway-translation", feature = "provider-deepseek"))]
#[tokio::test]
async fn gateway_builder_constructs_deepseek_context_cache_profile() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_context_cache_model(
        "deepseek",
        &deepseek_config("deepseek-reasoner"),
        &deepseek_env(),
    )
    .await?
    .expect("deepseek context cache builder should return a model");

    assert_eq!(model.provider(), "deepseek");
    assert_eq!(model.model_id(), "deepseek-reasoner");
    assert!(model.context_cache_profile().supports_caching());
    assert!(
        model
            .context_cache_profile()
            .supports_mode(ditto_llm::ContextCacheMode::Passive)
    );
    Ok(())
}
