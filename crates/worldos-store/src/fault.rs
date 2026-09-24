//! Deterministic, in-process I/O-fault injection for
//! [`SqliteStore`][crate::SqliteStore].
//!
//! Compiled only for tests and the `fault-injection` feature; production
//! builds carry neither the injector field nor any call sites, so the
//! default save/load path is unchanged.
//!
//! The seam models failures at the store's real boundaries — inside the
//! save transaction (each staged write phase), at the commit boundary
//! (where a write/fsync failure would land), immediately after commit
//! (a lost acknowledgment: the effect is durable but the caller sees an
//! error), and inside the load row streams. [`FaultPoint`] names map
//! one-to-one onto those phases. Faults are one-shot: an armed point
//! fires exactly once, on the next pass through it, then disarms.
//!
//! This complements `tests/crash_recovery.rs`, which crashes a real
//! child process around commits. Injection here exercises the error
//! paths a dying `Write`/`fsync` would produce without needing a custom
//! SQLite VFS — page-level torn writes below the transaction layer
//! remain SQLite's own atomicity guarantee and are out of scope.

use std::cell::RefCell;

use crate::error::StoreError;

/// A boundary inside [`SqliteStore`][crate::SqliteStore] at which an
/// injected fault fires.
///
/// `Save*` points sit inside (or just outside) the save transaction;
/// `Load*` points sit inside the read path. Firing returns a
/// [`StoreError::Io`] to the caller, exactly as a failed `write`/`read`/
/// `fsync` would surface through rusqlite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaultPoint {
    /// `save`: project rows deleted inside the transaction, no
    /// replacement rows staged yet — the deepest tear the transaction
    /// must roll back.
    SaveAfterWipe,
    /// `save`: meta rows staged; object/relation/journal rows not yet.
    SaveAfterMeta,
    /// `save`: object rows staged; relation/journal rows not yet.
    SaveAfterObjects,
    /// `save`: relation rows staged; journal rows not yet.
    SaveAfterRelations,
    /// `save`: journal (`transactions` table) rows staged; commit not
    /// yet issued. Index updates ride the same transaction — a fault
    /// here covers journal-append and index-update atomicity together.
    SaveAfterJournal,
    /// `save`: every write staged, `commit()` not yet issued — models a
    /// write/fsync failure exactly at the commit boundary.
    SaveBeforeCommit,
    /// `save`: `commit()` returned `Ok` — models a lost acknowledgment
    /// or post-commit sync failure. The effect IS durable; the caller
    /// still sees an error. Reopen must find the NEW snapshot.
    SaveAfterCommit,
    /// `load`: meta rows read; object/relation/journal rows not yet.
    LoadAfterMeta,
    /// `load`: mid object-row stream.
    LoadObjects,
    /// `load`: mid relation-row stream.
    LoadRelations,
    /// `load`: mid journal (`transactions`) row stream.
    LoadJournal,
}

/// Ordered set of armed [`FaultPoint`]s. Each armed point fires once —
/// the next time execution passes it — then disarms, so "fail once then
/// recover" needs no explicit reset.
///
/// Interior mutability keeps `check` callable from `load(&self)`;
/// `RefCell` preserves `Send` (but not `Sync`), matching the
/// `ProjectStore: Send` bound.
#[derive(Debug, Default)]
pub struct FaultInjector {
    armed: RefCell<Vec<FaultPoint>>,
}

impl FaultInjector {
    /// Arm `point`: the next pass through it returns an injected error.
    /// Arm the same point twice to make it fire twice.
    pub fn arm(&mut self, point: FaultPoint) {
        self.armed.borrow_mut().push(point);
    }

    /// True while at least one fault is still armed — lets tests prove
    /// an armed fault actually fired (vs. never being reached).
    pub fn is_armed(&self) -> bool {
        !self.armed.borrow().is_empty()
    }

    /// Number of faults still armed.
    pub fn armed_count(&self) -> usize {
        self.armed.borrow().len()
    }

    /// Drop every armed fault without firing it.
    pub fn clear(&mut self) {
        self.armed.borrow_mut().clear();
    }

    /// Fire iff `point` is armed; disarms on fire (one-shot).
    pub(crate) fn check(&self, point: FaultPoint) -> Result<(), StoreError> {
        let mut armed = self.armed.borrow_mut();
        if let Some(i) = armed.iter().position(|p| *p == point) {
            armed.remove(i);
            drop(armed);
            return Err(StoreError::Io(std::io::Error::other(format!(
                "injected fault at {point:?}"
            ))));
        }
        Ok(())
    }
}
