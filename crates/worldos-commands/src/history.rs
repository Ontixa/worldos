//! Universal history: append-only transaction log with linear undo/redo.

use crate::txn::TransactionRecord;
use serde::{Deserialize, Serialize};
use worldos_kernel::project::Project;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct History {
    pub records: Vec<TransactionRecord>,
    /// Number of applied transactions. `records[cursor..]` are undone.
    pub cursor: usize,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a committed transaction. Drops any redo tail.
    pub fn push(&mut self, record: TransactionRecord) {
        self.records.truncate(self.cursor);
        // clear `undone` flags on truncated tail implicitly by removal
        self.records.push(record);
        self.cursor = self.records.len();
    }

    pub fn next_index(&self) -> u64 {
        self.records.len() as u64
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }
    pub fn can_redo(&self) -> bool {
        self.cursor < self.records.len()
    }

    /// Revert the most recent applied transaction. Returns its id.
    pub fn undo(
        &mut self,
        project: &mut Project,
    ) -> Option<worldos_kernel::ids::TransactionId> {
        if self.cursor == 0 {
            return None;
        }
        let idx = self.cursor - 1;
        let rec = &self.records[idx];
        for op in rec.ops.iter().rev() {
            let _ = project.apply(op, false);
        }
        let id = rec.id;
        self.records[idx].undone = true;
        self.cursor = idx;
        Some(id)
    }

    /// Re-apply the most recently undone transaction.
    pub fn redo(
        &mut self,
        project: &mut Project,
    ) -> Option<worldos_kernel::ids::TransactionId> {
        if self.cursor >= self.records.len() {
            return None;
        }
        let rec = &self.records[self.cursor];
        for op in rec.ops.iter() {
            let _ = project.apply(op, true);
        }
        let id = rec.id;
        self.records[self.cursor].undone = false;
        self.cursor += 1;
        Some(id)
    }

    /// Rebuild project state by replaying all ops from scratch.
    /// Foundation for branching/replay and a consistency check.
    pub fn replay(&self, project: &mut Project) {
        project.objects.clear();
        project.relations.clear();
        for rec in &self.records[..self.cursor] {
            for op in &rec.ops {
                let _ = project.apply(op, true);
            }
        }
    }
}
