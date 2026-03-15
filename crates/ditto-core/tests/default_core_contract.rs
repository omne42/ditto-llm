use ditto_core::runtime_registry::builtin_runtime_registry_catalog;

fn is_default_core_build() -> bool {
    !(cfg!(feature = "provider-openai")
        || cfg!(feature = "provider-anthropic")
        || cfg!(feature = "provider-google")
        || cfg!(feature = "provider-cohere")
        || cfg!(feature = "provider-bedrock")
        || cfg!(feature = "provider-vertex")
        || cfg!(feature = "provider-bailian")
        || cfg!(feature = "provider-deepseek")
        || cfg!(feature = "provider-doubao")
        || cfg!(feature = "provider-hunyuan")
        || cfg!(feature = "provider-kimi")
        || cfg!(feature = "provider-minimax")
        || cfg!(feature = "provider-openrouter")
        || cfg!(feature = "provider-qianfan")
        || cfg!(feature = "provider-xai")
        || cfg!(feature = "provider-zhipu")
        || cfg!(feature = "cap-embedding")
        || cfg!(any(
            feature = "cap-image-generation",
            feature = "cap-image-edit"
        ))
        || cfg!(any(
            feature = "cap-audio-transcription",
            feature = "cap-audio-speech"
        ))
        || cfg!(feature = "cap-moderation")
        || cfg!(feature = "cap-rerank")
        || cfg!(feature = "cap-batch")
        || cfg!(feature = "cap-realtime"))
}

#[test]
fn default_core_exposes_only_generic_openai_compatible_llm_surface() {
    let registry = builtin_runtime_registry_catalog();
    let presets = registry.provider_presets();
    let preset = presets
        .iter()
        .find(|preset| preset.provider == "openai-compatible")
        .expect("openai-compatible preset should exist");
    assert_eq!(preset.provider, "openai-compatible");

    let summaries = registry.provider_capability_summaries();
    let summary = summaries
        .iter()
        .find(|summary| summary.provider == "openai-compatible")
        .expect("openai-compatible summary should exist");
    assert!(
        summary
            .capabilities
            .iter()
            .any(|capability| capability.as_str() == "llm")
    );

    if is_default_core_build() {
        assert_eq!(presets.len(), 1);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summary.capabilities.len(), 1);
    }
}
