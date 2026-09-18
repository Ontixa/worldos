//! Artifact store errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid artifact reference: {0}")]
    InvalidRef(String),
    #[error("artifact not found: {0}")]
    NotFound(String),
    #[error("artifact corrupt: expected {expected}, stored bytes hash to {actual}")]
    Corrupt { expected: String, actual: String },
}
