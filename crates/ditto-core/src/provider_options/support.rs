//! Warning-surface adapters for canonical provider-options values.
//!
//! This layer depends on contract warnings, but the schema core above it does
//! not.

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
use super::ProviderOptions;
#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
use crate::contracts::Warning;

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProviderOptionsSupport {
    pub reasoning_effort: bool,
    pub response_format: bool,
    pub parallel_tool_calls: bool,
}

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
impl ProviderOptionsSupport {
    pub(crate) const NONE: Self = Self {
        reasoning_effort: false,
        response_format: false,
        parallel_tool_calls: false,
    };
}

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "google",
    feature = "vertex",
))]
pub(crate) fn warn_unsupported_provider_options(
    provider_display: &str,
    provider_options: &ProviderOptions,
    supported: ProviderOptionsSupport,
    warnings: &mut Vec<Warning>,
) {
    if provider_options.reasoning_effort.is_some() && !supported.reasoning_effort {
        warnings.push(Warning::Unsupported {
            feature: "reasoning_effort".to_string(),
            details: Some(format!(
                "{provider_display} does not support reasoning_effort"
            )),
        });
    }
    if provider_options.response_format.is_some() && !supported.response_format {
        warnings.push(Warning::Unsupported {
            feature: "response_format".to_string(),
            details: Some(format!(
                "{provider_display} does not support response_format"
            )),
        });
    }
    if provider_options.parallel_tool_calls == Some(true) && !supported.parallel_tool_calls {
        warnings.push(Warning::Unsupported {
            feature: "parallel_tool_calls".to_string(),
            details: Some(format!(
                "{provider_display} does not support parallel_tool_calls"
            )),
        });
    }
}
