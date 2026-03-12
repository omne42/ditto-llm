use ditto_llm::contracts::{CapabilityKind, OperationKind};

#[test]
fn runtime_registry_exposes_openai_compatible_llm_binding() {
    let snapshot = ditto_llm::runtime_registry::builtin_runtime_registry();
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

#[cfg(feature = "provider-google")]
#[test]
fn runtime_registry_exposes_generated_google_models() {
    let snapshot = ditto_llm::runtime_registry::builtin_runtime_registry();
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
