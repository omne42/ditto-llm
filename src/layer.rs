use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;
use crate::model::{LanguageModel, StreamResult};
use crate::types::{GenerateRequest, GenerateResponse};

#[async_trait]
pub trait LanguageModelLayer: Send + Sync {
    async fn generate(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> Result<GenerateResponse>;

    async fn stream(
        &self,
        inner: &dyn LanguageModel,
        request: GenerateRequest,
    ) -> Result<StreamResult>;
}

#[derive(Clone)]
pub struct LayeredLanguageModel {
    inner: Arc<dyn LanguageModel>,
    layer: Arc<dyn LanguageModelLayer>,
}

impl LayeredLanguageModel {
    pub fn new<L>(inner: Arc<dyn LanguageModel>, layer: L) -> Self
    where
        L: LanguageModelLayer + 'static,
    {
        Self {
            inner,
            layer: Arc::new(layer),
        }
    }

    pub fn with_layer<L>(self, layer: L) -> Self
    where
        L: LanguageModelLayer + 'static,
    {
        Self::new(Arc::new(self), layer)
    }

    pub fn inner(&self) -> &Arc<dyn LanguageModel> {
        &self.inner
    }
}

#[async_trait]
impl LanguageModel for LayeredLanguageModel {
    fn provider(&self) -> &str {
        self.inner.provider()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        self.layer.generate(self.inner.as_ref(), request).await
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        self.layer.stream(self.inner.as_ref(), request).await
    }
}

pub trait LanguageModelLayerExt: LanguageModel + Sized + 'static {
    fn layer<L>(self, layer: L) -> LayeredLanguageModel
    where
        L: LanguageModelLayer + 'static,
    {
        LayeredLanguageModel::new(Arc::new(self), layer)
    }
}

impl<T> LanguageModelLayerExt for T where T: LanguageModel + Sized + 'static {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::StreamExt;
    use futures_util::stream;

    use super::*;
    use crate::types::{FinishReason, StreamChunk, Usage, Warning};

    #[derive(Clone)]
    struct FakeModel {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LanguageModel for FakeModel {
        fn provider(&self) -> &str {
            "fake"
        }

        fn model_id(&self) -> &str {
            "fake-model"
        }

        async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(GenerateResponse {
                content: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                warnings: Vec::new(),
                provider_metadata: None,
            })
        }

        async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream::iter([Ok(StreamChunk::Warnings {
                warnings: vec![Warning::Unsupported {
                    feature: "test".to_string(),
                    details: None,
                }],
            })])))
        }
    }

    struct CountingLayer {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LanguageModelLayer for CountingLayer {
        async fn generate(
            &self,
            inner: &dyn LanguageModel,
            request: GenerateRequest,
        ) -> Result<GenerateResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            inner.generate(request).await
        }

        async fn stream(
            &self,
            inner: &dyn LanguageModel,
            request: GenerateRequest,
        ) -> Result<StreamResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            inner.stream(request).await
        }
    }

    #[tokio::test]
    async fn layered_model_calls_inner() -> Result<()> {
        let inner_calls = Arc::new(AtomicUsize::new(0));
        let layer_calls = Arc::new(AtomicUsize::new(0));

        let model = FakeModel {
            calls: Arc::clone(&inner_calls),
        }
        .layer(CountingLayer {
            calls: Arc::clone(&layer_calls),
        });

        model.generate(Vec::new().into()).await?;
        let mut stream = model.stream(Vec::new().into()).await?;
        let _ = stream.next().await.transpose()?;

        assert_eq!(layer_calls.load(Ordering::SeqCst), 2);
        assert_eq!(inner_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
