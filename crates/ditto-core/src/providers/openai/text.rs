use std::sync::Arc;

use async_trait::async_trait;

use super::OpenAI;
use super::OpenAIChatCompletions;
use crate::config::{Env, ProviderApi, ProviderConfig};
use crate::contracts::OperationKind;
use crate::contracts::{GenerateRequest, GenerateResponse};
use crate::error::Result;
use crate::llm_core::model::{LanguageModel, StreamResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAITextSurface {
    Responses,
    ChatCompletions,
}

type OpenAiTextOperationSupportResolver = dyn Fn(&str, OperationKind) -> bool + Send + Sync;

#[derive(Clone)]
pub struct OpenAITextModel {
    responses: OpenAI,
    chat_completions: OpenAIChatCompletions,
    preferred_surface: Option<OpenAITextSurface>,
    operation_support_resolver: Arc<OpenAiTextOperationSupportResolver>,
}

impl OpenAITextModel {
    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        let preferred_surface = preferred_surface_from_config(config.upstream_api)?;
        Ok(Self {
            responses: OpenAI::from_config(config, env).await?,
            chat_completions: OpenAIChatCompletions::from_config(config, env).await?,
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

        Err(crate::invalid_response!(
            "error_detail.openai.text_model_no_supported_surface",
            "model" => model
        ))
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
                    Err(crate::invalid_response!(
                        "error_detail.openai.text_model_responses_unsupported",
                        "model" => model
                    ))
                }
            }
            OpenAITextSurface::ChatCompletions => {
                if self.supports_operation(model, OperationKind::CHAT_COMPLETION)? {
                    Ok(surface)
                } else {
                    Err(crate::invalid_response!(
                        "error_detail.openai.text_model_chat_completions_unsupported",
                        "model" => model
                    ))
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
        Some(ProviderApi::GeminiGenerateContent) | Some(ProviderApi::AnthropicMessages) => Err(
            crate::invalid_response!("error_detail.openai.text_model_non_openai_upstream_api"),
        ),
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
        }
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        let model = self.resolve_model(&request)?;
        match self.surface_for_model(model)? {
            OpenAITextSurface::Responses => self.responses.stream(request).await,
            OpenAITextSurface::ChatCompletions => self.chat_completions.stream(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selects_surface_from_catalog() -> crate::error::Result<()> {
        let model = OpenAITextModel {
            responses: OpenAI::new("sk-test").with_model("gpt-5"),
            chat_completions: OpenAIChatCompletions::new("sk-test").with_model("gpt-5"),
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
            model.surface_for_model("computer-use-preview")?,
            OpenAITextSurface::Responses
        );
        Ok(())
    }

    #[test]
    fn explicit_surface_rejects_catalog_mismatch() {
        let model = OpenAITextModel {
            responses: OpenAI::new("sk-test").with_model("gpt-5"),
            chat_completions: OpenAIChatCompletions::new("sk-test").with_model("gpt-5"),
            preferred_surface: None,
            operation_support_resolver: Arc::new(|model, operation| {
                crate::runtime_registry::builtin_runtime_registry_catalog()
                    .provider_supports_operation("openai", model, operation)
            }),
        };

        let err = model
            .ensure_surface_supported("computer-use-preview", OpenAITextSurface::ChatCompletions)
            .expect_err("response-only model should reject chat/completions");
        match err {
            crate::error::DittoError::InvalidResponse(message) => {
                assert_eq!(
                    message.as_catalog().map(|message| message.code()),
                    Some("error_detail.openai.text_model_chat_completions_unsupported")
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
