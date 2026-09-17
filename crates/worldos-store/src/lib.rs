//! # worldos-store
//!
//! Persistence behind the `ProjectStore` abstraction. Ships with an
//! in-memory store and the SQLite `.worldos` file format.

pub mod error;
pub mod snapshot;
pub mod sqlite;
pub mod store;

pub use error::StoreError;
pub use snapshot::{Snapshot, FORMAT_VERSION};
pub use sqlite::SqliteStore;
pub use store::{MemoryStore, ProjectStore};
