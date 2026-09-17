//! `ProjectStore`: the storage abstraction every backend implements.
//!
//! Implementations: [`crate::memory::MemoryStore`] (tests/ephemeral),
//! [`crate::sqlite::SqliteStore`] (the `.worldos` file). Later: cloud,
//! browser IndexedDB — without changing call sites.

use crate::error::StoreError;
use crate::snapshot::Snapshot;

pub trait ProjectStore: Send {
    /// True when the backing store already contains a project.
    fn exists(&self) -> bool;
    /// Load the full snapshot. Fails cleanly on missing/corrupt data.
    fn load(&self) -> Result<Snapshot, StoreError>;
    /// Persist the full snapshot atomically.
    fn save(&mut self, snapshot: &Snapshot) -> Result<(), StoreError>;
}

/// In-memory store — used by tests and as the "unsaved project" backend.
#[derive(Debug, Default)]
pub struct MemoryStore {
    snapshot: Option<Snapshot>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ProjectStore for MemoryStore {
    fn exists(&self) -> bool {
        self.snapshot.is_some()
    }
    fn load(&self) -> Result<Snapshot, StoreError> {
        self.snapshot
            .clone()
            .ok_or_else(|| StoreError::NotFound("memory".into()))
    }
    fn save(&mut self, snapshot: &Snapshot) -> Result<(), StoreError> {
        self.snapshot = Some(snapshot.clone());
        Ok(())
    }
}
