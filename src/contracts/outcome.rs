use serde::{Deserialize, Serialize};

// CONTRACTS-OUTCOME-OWNER: machine-readable finish/usage/warning semantics live
// under `contracts`, not `types`, because providers and gateway share them as
// unified cross-surface outcome contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

impl Usage {
    pub fn merge_total(&mut self) {
        if self.total_tokens.is_some() {
            return;
        }
        if let (Some(input), Some(output)) = (self.input_tokens, self.output_tokens) {
            self.total_tokens = Some(input.saturating_add(output));
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Warning {
    Unsupported {
        feature: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    Clamped {
        parameter: String,
        original: f32,
        clamped_to: f32,
    },
    Compatibility {
        feature: String,
        details: String,
    },
    Other {
        message: String,
    },
}
