//! Command-layer errors.

use thiserror::Error;
use worldos_kernel::KernelError;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("invalid input for {command}: {}", .errors.join("; "))]
    Validation { command: String, errors: Vec<String> },
    #[error("permission denied: `{perm}` required for {command}")]
    PermissionDenied { command: String, perm: String },
    #[error("command failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
}
