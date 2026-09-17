//! Capability-layer errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("unknown capability: {0}")]
    Unknown(String),
    #[error("permission denied: `{perm}` required for {capability}")]
    PermissionDenied { capability: String, perm: String },
    #[error("invalid input for {capability}: {}", .errors.join("; "))]
    Validation {
        capability: String,
        errors: Vec<String>,
    },
    #[error("capability failed: {0}")]
    Failed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
}
