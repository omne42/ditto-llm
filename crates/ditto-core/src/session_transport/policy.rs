use serde::{Deserialize, Serialize};

use super::sse::SseLimits;

// SESSION-TRANSPORT-POLICY-OWNER: session/frame limits live here as explicit
// session transport semantics.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionTransportPolicy {
    pub sse: SseLimits,
}
