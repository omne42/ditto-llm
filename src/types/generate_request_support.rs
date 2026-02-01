use crate::types::{GenerateRequest, Warning};

#[derive(Debug, Clone, Copy)]
pub(crate) struct GenerateRequestSupport {
    pub seed: bool,
    pub penalties: bool,
    pub logprobs: bool,
    pub user: bool,
}

impl GenerateRequestSupport {
    pub(crate) const NONE: Self = Self {
        seed: false,
        penalties: false,
        logprobs: false,
        user: false,
    };
}

pub(crate) fn warn_unsupported_generate_request_options(
    provider_display: &str,
    request: &GenerateRequest,
    supported: GenerateRequestSupport,
    warnings: &mut Vec<Warning>,
) {
    if request.seed.is_some() && !supported.seed {
        warnings.push(Warning::Unsupported {
            feature: "seed".to_string(),
            details: Some(format!("{provider_display} does not support seed")),
        });
    }

    if request.presence_penalty.is_some() && !supported.penalties {
        warnings.push(Warning::Unsupported {
            feature: "presence_penalty".to_string(),
            details: Some(format!(
                "{provider_display} does not support presence_penalty"
            )),
        });
    }

    if request.frequency_penalty.is_some() && !supported.penalties {
        warnings.push(Warning::Unsupported {
            feature: "frequency_penalty".to_string(),
            details: Some(format!(
                "{provider_display} does not support frequency_penalty"
            )),
        });
    }

    if request.logprobs == Some(true) && !supported.logprobs {
        warnings.push(Warning::Unsupported {
            feature: "logprobs".to_string(),
            details: Some(format!("{provider_display} does not support logprobs")),
        });
    }

    if request.top_logprobs.is_some() && !supported.logprobs {
        warnings.push(Warning::Unsupported {
            feature: "top_logprobs".to_string(),
            details: Some(format!("{provider_display} does not support top_logprobs")),
        });
    }

    if request
        .user
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .is_some()
        && !supported.user
    {
        warnings.push(Warning::Unsupported {
            feature: "user".to_string(),
            details: Some(format!("{provider_display} does not support user")),
        });
    }
}
