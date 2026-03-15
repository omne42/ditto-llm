use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::core::{StreamObjectFinal, StreamObjectState};
use crate::error::{DittoError, Result};

fn stream_object_state_lock_poisoned() -> DittoError {
    crate::invalid_response!("error_detail.stream.object_state_lock_poisoned")
}

fn stream_object_failed(error: &str) -> DittoError {
    crate::invalid_response!("error_detail.stream.object_failed", "error" => error)
}

fn final_object_deserialize_failed(error: impl std::fmt::Display) -> DittoError {
    crate::invalid_response!(
        "error_detail.object.final_deserialize_failed",
        "error" => error.to_string()
    )
}

#[derive(Clone)]
pub struct StreamObjectHandle {
    pub(super) state: Arc<Mutex<StreamObjectState>>,
}

impl StreamObjectHandle {
    pub fn is_done(&self) -> bool {
        self.state.lock().map(|s| s.done).unwrap_or(false)
    }

    pub fn final_json(&self) -> Result<Option<Value>> {
        let state = self
            .state
            .lock()
            .map_err(|_| stream_object_state_lock_poisoned())?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(stream_object_failed(err));
        }
        Ok(state.final_object.clone())
    }

    pub fn final_object<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        self.final_json()?
            .map(|value| {
                serde_json::from_value::<T>(value).map_err(final_object_deserialize_failed)
            })
            .transpose()
    }

    pub fn final_summary(&self) -> Result<Option<StreamObjectFinal>> {
        let state = self
            .state
            .lock()
            .map_err(|_| stream_object_state_lock_poisoned())?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(stream_object_failed(err));
        }
        let Some(object) = state.final_object.clone() else {
            return Ok(None);
        };
        let mut usage = state.usage.clone();
        usage.merge_total();
        Ok(Some(StreamObjectFinal {
            object,
            response_id: state.response_id.clone(),
            warnings: state.warnings.clone(),
            finish_reason: state.finish_reason,
            usage,
        }))
    }
}
