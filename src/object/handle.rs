#[derive(Clone)]
pub struct StreamObjectHandle {
    state: Arc<Mutex<StreamObjectState>>,
}

impl StreamObjectHandle {
    pub fn is_done(&self) -> bool {
        self.state.lock().map(|s| s.done).unwrap_or(false)
    }

    pub fn final_json(&self) -> Result<Option<Value>> {
        let state = self.state.lock().map_err(|_| {
            DittoError::InvalidResponse("stream object state lock is poisoned".to_string())
        })?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(DittoError::InvalidResponse(err.to_string()));
        }
        Ok(state.final_object.clone())
    }

    pub fn final_object<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        self.final_json()?
            .map(|value| {
                serde_json::from_value::<T>(value).map_err(|err| {
                    DittoError::InvalidResponse(format!(
                        "failed to deserialize final object: {err}"
                    ))
                })
            })
            .transpose()
    }

    pub fn final_summary(&self) -> Result<Option<StreamObjectFinal>> {
        let state = self.state.lock().map_err(|_| {
            DittoError::InvalidResponse("stream object state lock is poisoned".to_string())
        })?;
        if !state.done {
            return Ok(None);
        }
        if let Some(err) = state.final_error.as_deref() {
            return Err(DittoError::InvalidResponse(err.to_string()));
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
