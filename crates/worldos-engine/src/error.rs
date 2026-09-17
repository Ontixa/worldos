//! Engine errors — unified error surface for all interfaces.

use thiserror::Error;
use worldos_capability::CapabilityError;
use worldos_commands::CommandError;
use worldos_kernel::KernelError;
use worldos_store::StoreError;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error("no transaction is open")]
    NoOpenTransaction,
    #[error("a transaction is already open")]
    TransactionAlreadyOpen,
    #[error("cannot {action} while a transaction is open")]
    TransactionOpen { action: &'static str },
    #[error("project has no backing file (use save_as)")]
    NoBackingFile,
    #[error("{0}")]
    Other(String),
}
