//! # worldos-store
//!
//! Persistence behind the `ProjectStore` abstraction. Ships with an
//! in-memory store and the SQLite `.worldos` file format.

pub mod error;
#[cfg(any(test, feature = "fault-injection"))]
pub mod fault;
pub mod snapshot;
pub mod sqlite;
pub mod store;

pub use error::StoreError;
#[cfg(any(test, feature = "fault-injection"))]
pub use fault::{FaultInjector, FaultPoint};
pub use snapshot::{FORMAT_VERSION, Snapshot};
pub use sqlite::SqliteStore;
pub use store::{MemoryStore, ProjectStore};
