#![cfg(feature = "provider-minimax")]

use std::collections::BTreeSet;

use ditto_llm::{
    CapabilityKind, Env, OperationKind, ProviderAuth, ProviderConfig, ProviderProtocolFamily,
    builtin_registry,
};

fn minimax_env() -> Env {
    Env::parse_dotenv("MINIMAX_API_KEY=sk-test\n")
}

fn minimax_config(default_model: &str) -> ProviderConfig {
    ProviderConfig {
        provider: Some("minimax".to_string()),
        base_url: Some("https://api.minimaxi.com".to_string()),
        default_model: Some(default_model.to_string()),
        auth: Some(ProviderAuth::ApiKeyEnv {
            keys: vec!["MINIMAX_API_KEY".to_string()],
        }),
        ..ProviderConfig::default()
    }
}

#[test]
fn minimax_catalog_runtime_spec_includes_context_cache() {
    let plugin = builtin_registry()
        .plugin("minimax")
        .expect("minimax plugin should be available");
    let runtime_spec = plugin.runtime_spec();
    let actual = runtime_spec
        .capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(runtime_spec.protocol_family, ProviderProtocolFamily::Mixed);
    assert_eq!(
        actual,
        BTreeSet::from([
            CapabilityKind::AUDIO_SPEECH.as_str(),
            CapabilityKind::AUDIO_VOICE_CLONE.as_str(),
            CapabilityKind::AUDIO_VOICE_DESIGN.as_str(),
            CapabilityKind::CONTEXT_CACHE.as_str(),
            CapabilityKind::IMAGE_GENERATION.as_str(),
            CapabilityKind::LLM.as_str(),
            CapabilityKind::MUSIC_GENERATION.as_str(),
            CapabilityKind::VIDEO_GENERATION.as_str(),
        ])
    );

    let resolution = plugin.capability_resolution(Some("MiniMax-M2"));
    assert!(resolution.effective_supports(CapabilityKind::LLM));
    assert!(resolution.effective_supports(CapabilityKind::CONTEXT_CACHE));
    assert!(
        builtin_registry()
            .resolve("minimax", "MiniMax-M2", OperationKind::CHAT_COMPLETION)
            .is_some()
    );
    assert!(
        builtin_registry()
            .resolve("minimax", "MiniMax-M2", OperationKind::CONTEXT_CACHE)
            .is_some()
    );
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-minimax",
    feature = "openai-compatible"
))]
#[tokio::test]
async fn gateway_builder_constructs_minimax_context_cache_profile() -> ditto_llm::Result<()> {
    let model = ditto_llm::gateway::translation::build_context_cache_model(
        "minimax",
        &minimax_config("MiniMax-M2"),
        &minimax_env(),
    )
    .await?
    .expect("minimax context cache builder should return a model");

    assert_eq!(model.provider(), "minimax");
    assert_eq!(model.model_id(), "MiniMax-M2");
    assert!(model.context_cache_profile().supports_caching());
    assert!(
        model
            .context_cache_profile()
            .supports_mode(ditto_llm::ContextCacheMode::Passive)
    );
    assert!(
        model
            .context_cache_profile()
            .supports_mode(ditto_llm::ContextCacheMode::AnthropicCompatible)
    );
    Ok(())
}
