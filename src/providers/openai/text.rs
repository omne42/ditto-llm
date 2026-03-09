use async_trait::async_trait;

#[cfg(feature = "openai-compatible")]
use super::OpenAIChatCompletions;
use super::{OpenAI, OpenAICompletionsLegacy};
use crate::catalog::OperationKind;
use crate::config::{Env, ProviderApi, ProviderConfig};
use crate::model::{LanguageModel, StreamResult};
use crate::{DittoError, GenerateRequest, GenerateResponse, Result, builtin_registry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAITextSurface {
    Responses,
    ChatCompletions,
    LegacyCompletions,
}

#[derive(Clone)]
pub struct OpenAITextModel {
    responses: OpenAI,
    #[cfg(feature = "openai-compatible")]
    chat_completions: OpenAIChatCompletions,
    legacy_completions: OpenAICompletionsLegacy,
    preferred_surface: Option<OpenAITextSurface>,
}

impl OpenAITextModel {
    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        let preferred_surface = preferred_surface_from_config(config.upstream_api)?;
        Ok(Self {
            responses: OpenAI::from_config(config, env).await?,
            #[cfg(feature = "openai-compatible")]
            chat_completions: OpenAIChatCompletions::from_config(config, env).await?,
            legacy_completions: OpenAICompletionsLegacy::from_config(config, env).await?,
            preferred_surface,
        })
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        self.responses.resolve_model(request)
    }

    fn surface_for_model(&self, model: &str) -> Result<OpenAITextSurface> {
        if let Some(surface) = self.preferred_surface {
            return ensure_surface_supported(model, surface);
        }

        if builtin_registry()
            .resolve("openai", model, OperationKind::RESPONSE)
            .is_some()
        {
            return Ok(OpenAITextSurface::Responses);
        }

        if builtin_registry()
            .resolve("openai", model, OperationKind::CHAT_COMPLETION)
            .is_some()
        {
            return ensure_surface_supported(model, OpenAITextSurface::ChatCompletions);
        }

        if builtin_registry()
            .resolve("openai", model, OperationKind::TEXT_COMPLETION)
            .is_some()
        {
            return Ok(OpenAITextSurface::LegacyCompletions);
        }

        Err(DittoError::InvalidResponse(format!(
            "openai model {model} has no supported text invocation surface in the builtin catalog"
        )))
    }
}

fn preferred_surface_from_config(
    upstream_api: Option<ProviderApi>,
) -> Result<Option<OpenAITextSurface>> {
    match upstream_api {
        Some(ProviderApi::OpenaiResponses) => Ok(Some(OpenAITextSurface::Responses)),
        Some(ProviderApi::OpenaiChatCompletions) => {
            ensure_chat_surface_compiled()?;
            Ok(Some(OpenAITextSurface::ChatCompletions))
        }
        Some(ProviderApi::GeminiGenerateContent) | Some(ProviderApi::AnthropicMessages) => {
            Err(DittoError::InvalidResponse(
                "openai text model cannot be configured with a non-openai upstream_api".to_string(),
            ))
        }
        None => Ok(None),
    }
}

fn ensure_surface_supported(model: &str, surface: OpenAITextSurface) -> Result<OpenAITextSurface> {
    match surface {
        OpenAITextSurface::Responses => {
            if builtin_registry()
                .resolve("openai", model, OperationKind::RESPONSE)
                .is_some()
            {
                Ok(surface)
            } else {
                Err(DittoError::InvalidResponse(format!(
                    "openai model {model} does not support /v1/responses"
                )))
            }
        }
        OpenAITextSurface::ChatCompletions => {
            ensure_chat_surface_compiled()?;
            if builtin_registry()
                .resolve("openai", model, OperationKind::CHAT_COMPLETION)
                .is_some()
            {
                Ok(surface)
            } else {
                Err(DittoError::InvalidResponse(format!(
                    "openai model {model} does not support /v1/chat/completions"
                )))
            }
        }
        OpenAITextSurface::LegacyCompletions => {
            if builtin_registry()
                .resolve("openai", model, OperationKind::TEXT_COMPLETION)
                .is_some()
            {
                Ok(surface)
            } else {
                Err(DittoError::InvalidResponse(format!(
                    "openai model {model} does not support /v1/completions"
                )))
            }
        }
    }
}

fn ensure_chat_surface_compiled() -> Result<()> {
    #[cfg(feature = "openai-compatible")]
    {
        Ok(())
    }
    #[cfg(not(feature = "openai-compatible"))]
    {
        Err(DittoError::InvalidResponse(
            "ditto-llm built without openai-compatible feature required for openai chat/completions surface".to_string(),
        ))
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
            OpenAITextSurface::ChatCompletions => {
                #[cfg(feature = "openai-compatible")]
                {
                    self.chat_completions.generate(request).await
                }
                #[cfg(not(feature = "openai-compatible"))]
                {
                    let _ = request;
                    Err(DittoError::InvalidResponse(
                        "ditto-llm built without openai-compatible feature required for openai chat/completions surface".to_string(),
                    ))
                }
            }
            OpenAITextSurface::LegacyCompletions => self.legacy_completions.generate(request).await,
        }
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        let model = self.resolve_model(&request)?;
        match self.surface_for_model(model)? {
            OpenAITextSurface::Responses => self.responses.stream(request).await,
            OpenAITextSurface::ChatCompletions => {
                #[cfg(feature = "openai-compatible")]
                {
                    self.chat_completions.stream(request).await
                }
                #[cfg(not(feature = "openai-compatible"))]
                {
                    let _ = request;
                    Err(DittoError::InvalidResponse(
                        "ditto-llm built without openai-compatible feature required for openai chat/completions surface".to_string(),
                    ))
                }
            }
            OpenAITextSurface::LegacyCompletions => self.legacy_completions.stream(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selects_surface_from_catalog() -> crate::Result<()> {
        let model = OpenAITextModel {
            responses: OpenAI::new("sk-test").with_model("gpt-5"),
            #[cfg(feature = "openai-compatible")]
            chat_completions: OpenAIChatCompletions::new("sk-test").with_model("gpt-5"),
            legacy_completions: OpenAICompletionsLegacy::new("sk-test").with_model("gpt-5"),
            preferred_surface: None,
        };

        assert_eq!(
            model.surface_for_model("gpt-5")?,
            OpenAITextSurface::Responses
        );
        assert_eq!(
            model.surface_for_model("gpt-4")?,
            OpenAITextSurface::ChatCompletions
        );
        assert_eq!(
            model.surface_for_model("davinci-002")?,
            OpenAITextSurface::LegacyCompletions
        );
        Ok(())
    }

    #[test]
    fn explicit_surface_is_validated_against_catalog() {
        let err = ensure_surface_supported("gpt-5", OpenAITextSurface::LegacyCompletions)
            .expect_err("gpt-5 should reject legacy completions");
        assert!(err.to_string().contains("does not support /v1/completions"));
    }
}
