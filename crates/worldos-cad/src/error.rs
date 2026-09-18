//! CAD errors — kernel-agnostic.

use thiserror::Error;

use crate::types::ShapeId;

#[derive(Debug, Error)]
pub enum CadError {
    #[error("kernel error: {0}")]
    Kernel(String),
    #[error("unknown shape handle {0:?}")]
    UnknownShape(ShapeId),
    #[error("operation not supported by this kernel: {0}")]
    Unsupported(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
