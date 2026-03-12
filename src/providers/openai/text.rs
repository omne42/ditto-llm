use std::sync::Arc;

use async_trait::async_trait;

use super::OpenAIChatCompletions;
use super::{OpenAI, OpenAICompletionsLegacy};
use crate::config::{Env, ProviderApi, ProviderConfig};
use crate::contracts::OperationKind;
use crate::contracts::{GenerateRequest, GenerateResponse};
use crate::foundation::error::{DittoError, Result};
use crate::llm_core::model::{LanguageModel, StreamResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAITextSurface {
    Responses,
    ChatCompletions,
    LegacyCompletions,
}

type OpenAiTextOperationSupportResolver = dyn Fn(&str, OperationKind) -> bool + Send + Sync;

#[derive(Clone)]
pub struct OpenAITextModel {
    responses: OpenAI,
    chat_completions: OpenAIChatCompletions,
    legacy_completions: OpenAICompletionsLegacy,
    preferred_surface: Option<OpenAITextSurface>,
    operation_support_resolver: Arc<OpenAiTextOperationSupportResolver>,
}

impl OpenAITextModel {
    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        let preferred_surface = preferred_surface_from_config(config.upstream_api)?;
        Ok(Self {
            responses: OpenAI::from_config(config, env).await?,
            chat_completions: OpenAIChatCompletions::from_config(config, env).await?,
            legacy_completions: OpenAICompletionsLegacy::from_config(config, env).await?,
            preferred_surface,
            // OPENAI-TEXT-SELF-SUFFICIENT-CONSTRUCTOR: public constructors must
            // build a semantically complete adapter. Runtime may still override
            // this resolver internally, but direct users no longer get a
            // half-initialized object that fails only at first request.
            operation_support_resolver: default_operation_support_resolver(),
        })
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        self.responses.resolve_model(request)
    }

    fn surface_for_model(&self, model: &str) -> Result<OpenAITextSurface> {
        if let Some(surface) = self.preferred_surface {
            return self.ensure_surface_supported(model, surface);
        }

        if self.supports_operation(model, OperationKind::RESPONSE)? {
            return Ok(OpenAITextSurface::Responses);
        }

        if self.supports_operation(model, OperationKind::CHAT_COMPLETION)? {
            return self.ensure_surface_supported(model, OpenAITextSurface::ChatCompletions);
        }

        if self.supports_operation(model, OperationKind::TEXT_COMPLETION)? {
            return Ok(OpenAITextSurface::LegacyCompletions);
        }

        Err(DittoError::InvalidResponse(format!(
            "openai model {model} has no supported text invocation surface in the builtin catalog"
        )))
    }

    fn ensure_surface_supported(
        &self,
        model: &str,
        surface: OpenAITextSurface,
    ) -> Result<OpenAITextSurface> {
        match surface {
            OpenAITextSurface::Responses => {
                if self.supports_operation(model, OperationKind::RESPONSE)? {
                    Ok(surface)
                } else {
                    Err(DittoError::InvalidResponse(format!(
                        "openai model {model} does not support /v1/responses"
                    )))
                }
            }
            OpenAITextSurface::ChatCompletions => {
                if self.supports_operation(model, OperationKind::CHAT_COMPLETION)? {
                    Ok(surface)
                } else {
                    Err(DittoError::InvalidResponse(format!(
                        "openai model {model} does not support /v1/chat/completions"
                    )))
                }
            }
            OpenAITextSurface::LegacyCompletions => {
                if self.supports_operation(model, OperationKind::TEXT_COMPLETION)? {
                    Ok(surface)
                } else {
                    Err(DittoError::InvalidResponse(format!(
                        "openai model {model} does not support /v1/completions"
                    )))
                }
            }
        }
    }

    fn supports_operation(&self, model: &str, operation: OperationKind) -> Result<bool> {
        Ok((self.operation_support_resolver)(model, operation))
    }
}

fn default_operation_support_resolver() -> Arc<OpenAiTextOperationSupportResolver> {
    Arc::new(|model, operation| {
        crate::runtime_registry::builtin_runtime_registry_catalog()
            .provider_supports_operation("openai", model, operation)
    })
}

fn preferred_surface_from_config(
    upstream_api: Option<ProviderApi>,
) -> Result<Option<OpenAITextSurface>> {
    match upstream_api {
        Some(ProviderApi::OpenaiResponses) => Ok(Some(OpenAITextSurface::Responses)),
        Some(ProviderApi::OpenaiChatCompletions) => Ok(Some(OpenAITextSurface::ChatCompletions)),
        Some(ProviderApi::GeminiGenerateContent) | Some(ProviderApi::AnthropicMessages) => {
            Err(DittoError::InvalidResponse(
                "openai text model cannot be configured with a non-openai upstream_api".to_string(),
            ))
        }
        None => Ok(None),
    }
}

#[async_trait]
impl LanguageModel for OpenAITextModel {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        self.responses.model_id()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?;
        match self.surface_for_model(model)? {
            OpenAITextSurface::Responses => self.responses.generate(request).await,
            OpenAITextSurface::ChatCompletions => self.chat_completions.generate(request).await,
            OpenAITextSurface::LegacyCompletions => self.legacy_completions.generate(request).await,
        }
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        let model = self.resolve_model(&request)?;
        match self.surface_for_model(model)? {
            OpenAITextSurface::Responses => self.responses.stream(request).await,
            OpenAITextSurface::ChatCompletions => self.chat_completions.stream(request).await,
            OpenAITextSurface::LegacyCompletions => self.legacy_completions.stream(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selects_surface_from_catalog() -> crate::foundation::error::Result<()> {
        let model = OpenAITextModel {
            responses: OpenAI::new("sk-test").with_model("gpt-5"),
            chat_completions: OpenAIChatCompletions::new("sk-test").with_model("gpt-5"),
            legacy_completions: OpenAICompletionsLegacy::new("sk-test").with_model("gpt-5"),
            preferred_surface: None,
            operation_support_resolver: default_operation_support_resolver(),
        };

        assert_eq!(
            model.surface_for_model("gpt-5")?,
            OpenAITextSurface::Responses
        );
        assert_eq!(
            model.surface_for_model("gpt-4")?,
            OpenAITextSurface::Responses
        );
        assert_eq!(
            model.surface_for_model("davinci-002")?,
            OpenAITextSurface::LegacyCompletions
        );
        Ok(())
    }

    #[test]
    fn explicit_surface_is_validated_against_catalog() {
        let model = OpenAITextModel {
            responses: OpenAI::new("sk-test").with_model("gpt-5"),
            chat_completions: OpenAIChatCompletions::new("sk-test").with_model("gpt-5"),
            legacy_completions: OpenAICompletionsLegacy::new("sk-test").with_model("gpt-5"),
            preferred_surface: None,
            operation_support_resolver: Arc::new(|model, operation| {
                crate::runtime_registry::builtin_runtime_registry_catalog()
                    .provider_supports_operation("openai", model, operation)
            }),
        };

        let err = model
            .ensure_surface_supported("gpt-5", OpenAITextSurface::LegacyCompletions)
            .expect_err("gpt-5 should reject legacy completions");
        assert!(err.to_string().contains("does not support /v1/completions"));
    }
}
