//! # worldos-commands
//!
//! All project mutations flow through this crate: serializable command
//! envelopes → schema validation → handler execution → recorded `StateOp`s
//! inside a transaction → committed to the undoable history log.

pub mod builtin;
pub mod envelope;
pub mod error;
pub mod handler;
pub mod history;
pub mod schema;
pub mod txn;

pub use envelope::{CommandEnvelope, CommandReceipt, CommandRecord};
pub use error::CommandError;
pub use handler::{CommandContext, CommandHandler, CommandRegistry};
pub use history::History;
pub use schema::CommandSchema;
pub use txn::{Transaction, TransactionRecord};
