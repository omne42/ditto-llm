pub(crate) type OpenAiCompatibleProviderOptionsSchemaResult =
    crate::providers::openai_chat_completions_core::OpenAiChatCompletionsProviderOptionsSchemaResult;

pub(crate) fn apply_openai_compatible_provider_options_schema(
    family: crate::providers::openai_compat_profile::OpenAiProviderFamily,
    selected_provider_options: Option<serde_json::Value>,
    reserved_keys: &[&str],
    provider_options_context: &'static str,
) -> OpenAiCompatibleProviderOptionsSchemaResult {
    crate::providers::openai_chat_completions_core::apply_openai_chat_completions_provider_options_schema(
        family,
        selected_provider_options,
        reserved_keys,
        provider_options_context,
    )
}
