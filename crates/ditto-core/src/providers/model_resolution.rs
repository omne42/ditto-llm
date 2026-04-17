use crate::error::{DittoError, Result};

pub(crate) fn resolve_model_or_default<'a>(
    request_model: Option<&'a str>,
    default_model: &'a str,
    subject: &str,
    hint: &str,
) -> Result<&'a str> {
    if let Some(model) = request_model {
        return Ok(model);
    }
    if !default_model.trim().is_empty() {
        return Ok(default_model);
    }
    Err(DittoError::provider_model_missing(subject, hint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_model_wins_over_default() -> Result<()> {
        let model = resolve_model_or_default(
            Some(" request-model "),
            "default-model",
            "test-provider",
            "set a model",
        )?;
        assert_eq!(model, " request-model ");
        Ok(())
    }

    #[test]
    fn falls_back_to_default_model() -> Result<()> {
        let model =
            resolve_model_or_default(None, "default-model", "test-provider", "set a model")?;
        assert_eq!(model, "default-model");
        Ok(())
    }

    #[test]
    fn errors_when_no_request_or_default_model_exists() {
        let err = resolve_model_or_default(None, "   ", "test-provider", "set a model")
            .expect_err("missing model should error");
        assert!(
            err.to_string().contains("test-provider"),
            "unexpected error: {err}"
        );
    }
}
