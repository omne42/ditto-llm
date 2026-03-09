//! Provider-agnostic L0 language model core.

pub mod error {
    pub use crate::foundation::error::{DittoError, ProviderResolutionError, Result};
}

pub mod layer {
    pub use crate::layer::{LanguageModelLayer, LanguageModelLayerExt, LayeredLanguageModel};
}

pub mod model {
    pub use crate::model::{LanguageModel, StreamResult};
}

pub mod stream {
    pub use crate::stream::{
        AbortableStream, CollectedStream, LanguageModelExt, StreamAbortHandle, abortable_stream,
        collect_stream,
    };
}

pub use error::{DittoError, ProviderResolutionError, Result};
pub use layer::{LanguageModelLayer, LanguageModelLayerExt, LayeredLanguageModel};
pub use model::{LanguageModel, StreamResult};
pub use stream::{
    AbortableStream, CollectedStream, LanguageModelExt, StreamAbortHandle, abortable_stream,
    collect_stream,
};
