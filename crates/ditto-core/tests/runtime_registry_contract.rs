use ditto_core::contracts::{CapabilityKind, OperationKind};

#[test]
fn runtime_registry_exposes_openai_compatible_llm_binding() {
    let snapshot = ditto_core::runtime_registry::builtin_runtime_registry();
    let provider = snapshot
        .provider("openai-compatible")
        .expect("openai-compatible runtime registry entry should exist");

    assert_eq!(provider.provider(), "openai-compatible");
    assert!(
        provider
            .capabilities
            .contains(&CapabilityKind::LLM.as_str())
    );
    assert!(provider.capability_bindings.iter().any(|binding| {
        binding.capability == CapabilityKind::LLM.as_str()
            && binding
                .operations
                .contains(&OperationKind::CHAT_COMPLETION.as_str())
    }));

    let json = serde_json::to_value(&snapshot).expect("runtime registry should serialize");
    assert!(json.get("providers").is_some());
}

#[test]
fn runtime_registry_declares_model_truth_precedence() {
    use ditto_core::runtime_registry::{
        MODEL_TRUTH_PRECEDENCE, ModelTruthSource, model_truth_precedence,
    };

    assert_eq!(
        model_truth_precedence(),
        &[
            ModelTruthSource::UserConfig,
            ModelTruthSource::RuntimeRegistry,
            ModelTruthSource::BuiltinCatalog,
        ]
    );
    assert_eq!(
        MODEL_TRUTH_PRECEDENCE
            .iter()
            .map(|source| source.as_str())
            .collect::<Vec<_>>(),
        vec!["user-config", "runtime-registry", "builtin-catalog"]
    );
    assert_eq!(
        ModelTruthSource::BuiltinCatalog.role(),
        "Compiled fallback metadata layer, including generated provider catalog artifacts."
    );
}

#[cfg(feature = "provider-google")]
#[test]
fn runtime_registry_exposes_generated_google_models() {
    let snapshot = ditto_core::runtime_registry::builtin_runtime_registry();
    let provider = snapshot
        .provider("google")
        .expect("google runtime registry entry should exist");
    let model = provider
        .model("gemini-3.1-pro")
        .expect("google registry should expose generated gemini model");

    assert_eq!(model.provider(), "google");
    assert_eq!(model.model(), "gemini-3.1-pro");
    assert!(
        model
            .supported_operations
            .contains(&OperationKind::CHAT_COMPLETION.as_str())
    );
}
