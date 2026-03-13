#![cfg(feature = "provider-anthropic")]

use std::collections::BTreeSet;

use ditto_core::catalog::builtin_registry;
use ditto_core::config::{Env, ProviderConfig};
use ditto_core::contracts::{CapabilityKind, OperationKind, ProviderProtocolFamily};
use ditto_core::foundation::error::{DittoError, ProviderResolutionError};

fn anthropic_env() -> Env {
    Env::parse_dotenv("ANTHROPIC_API_KEY=sk-ant-test\n")
}

fn anthropic_config(default_model: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: Some("https://api.anthropic.com/v1".to_string()),
        default_model: Some(default_model.to_string()),
        ..ProviderConfig::default()
    }
}

#[test]
fn anthropic_catalog_runtime_spec_matches_enabled_capabilities() {
    let plugin = builtin_registry()
        .plugin("anthropic")
        .expect("anthropic plugin should be available");
    let runtime_spec = plugin.runtime_spec();
    let actual = runtime_spec
        .capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        runtime_spec.protocol_family,
        ProviderProtocolFamily::Anthropic
    );
    assert_eq!(actual, BTreeSet::from([CapabilityKind::LLM.as_str()]));
    assert!(
        plugin
            .capability_resolution(Some("claude-3-7-sonnet-20250219"))
            .effective_supports(CapabilityKind::LLM)
    );
    assert!(
        builtin_registry()
            .resolve(
                "anthropic",
                "claude-3-7-sonnet-20250219",
                OperationKind::CHAT_COMPLETION,
            )
            .is_some()
    );
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-anthropic",
    feature = "cap-llm"
))]
#[tokio::test]
async fn gateway_builder_constructs_anthropic_llm() -> ditto_core::foundation::error::Result<()> {
    let model = ditto_core::runtime::build_language_model(
        "anthropic",
        &anthropic_config("claude-3-7-sonnet-20250219"),
        &anthropic_env(),
    )
    .await?;

    assert_eq!(model.provider(), "anthropic");
    assert_eq!(model.model_id(), "claude-3-7-sonnet-20250219");
    Ok(())
}

#[cfg(all(
    feature = "gateway-translation",
    feature = "provider-anthropic",
    feature = "cap-embedding"
))]
#[tokio::test]
async fn gateway_builder_rejects_unimplemented_anthropic_embedding_capability() {
    let err = match ditto_core::runtime::build_embedding_model(
        "anthropic",
        &anthropic_config("claude-3-7-sonnet-20250219"),
        &anthropic_env(),
    )
    .await
    {
        Ok(_) => panic!("anthropic should reject embedding builder requests"),
        Err(err) => err,
    };

    assert!(matches!(
        err,
        DittoError::ProviderResolution(ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
            ref provider,
            ref capability,
            ..
        }) if provider == "anthropic" && capability == CapabilityKind::EMBEDDING.as_str()
    ));
}
