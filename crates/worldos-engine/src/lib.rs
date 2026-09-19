//! # worldos-engine
//!
//! The single orchestration point every interface shares: commands,
//! transactions, undo/redo history, capabilities, validation, snapshots
//! and semantic diff.

pub mod diff;
pub mod engine;
pub mod error;
pub mod save;

pub use diff::{DiffEntry, FieldChange, diff_projects};
pub use engine::Engine;
pub use error::EngineError;
pub use save::{SaveOptions, SaveStage};
