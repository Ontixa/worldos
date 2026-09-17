//! # worldos-store
//!
//! Persistence behind the `ProjectStore` abstraction. Ships with an
//! in-memory store and the SQLite `.worldos` file format.

pub mod error;
pub mod snapshot;
pub mod sqlite;
pub mod store;

pub use error::StoreError;
pub use snapshot::{FORMAT_VERSION, Snapshot};
pub use sqlite::SqliteStore;
pub use store::{MemoryStore, ProjectStore};
