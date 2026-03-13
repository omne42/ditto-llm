//! Provider-agnostic L0 language model core.

pub mod layer;
pub mod model;
pub mod stream;

// LLM-CORE-NO-TOPLEVEL-ALIASES: keep `llm_core` as the owner namespace, but do
// not duplicate `layer/model/stream` items onto a second top-level path such as
// `crate::llm_core::LanguageModel`. Callers should use the explicit submodule
// path that matches the actual owner.
