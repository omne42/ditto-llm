#[cfg(feature = "gateway")]
use omne_integrity_primitives::Sha256Hasher;

#[cfg(feature = "gateway")]
pub fn audit_chain_hash(
    prev_hash: Option<&str>,
    record: &crate::gateway::AuditLogRecord,
) -> String {
    let mut hasher = Sha256Hasher::new();
    if let Some(prev_hash) = prev_hash {
        hasher.update(prev_hash.as_bytes());
    }
    hasher.update(b"\n");
    if let Ok(serialized) = serde_json::to_vec(record) {
        hasher.update(&serialized);
    }
    hasher.finalize().to_string()
}
