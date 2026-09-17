//! Transactions: atomic, labeled groups of commands with recorded ops.

use crate::envelope::CommandRecord;
use serde::{Deserialize, Serialize};
use worldos_kernel::delta::StateOp;
use worldos_kernel::ids::{ObjectId, TransactionId};
use worldos_kernel::project::Project;
use worldos_kernel::{ActorId, TimestampMs};

/// An open transaction accumulating ops and command records.
pub struct Transaction {
    pub id: TransactionId,
    pub actor: ActorId,
    pub label: String,
    pub started_at: TimestampMs,
    pub ops: Vec<StateOp>,
    pub commands: Vec<CommandRecord>,
    /// Depth of nested run_sub calls — bookkeeping only.
    pub depth: usize,
}

impl Transaction {
    pub fn new(actor: ActorId, label: impl Into<String>) -> Self {
        Self {
            id: TransactionId::new(),
            actor,
            label: label.into(),
            started_at: worldos_kernel::now_ms(),
            ops: Vec::new(),
            commands: Vec::new(),
            depth: 0,
        }
    }

    /// Roll back all ops applied so far (failure or explicit abort).
    pub fn revert(&self, project: &mut Project) {
        for op in self.ops.iter().rev() {
            let _ = project.apply(op, false);
        }
    }

    pub fn affected_objects(&self) -> Vec<ObjectId> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                StateOp::SetObject { before, after } => after
                    .as_ref()
                    .map(|o| o.id)
                    .or_else(|| before.as_ref().map(|o| o.id)),
                _ => None,
            })
            .collect()
    }

    pub fn finish(self, index: u64) -> TransactionRecord {
        TransactionRecord {
            id: self.id,
            index,
            actor: self.actor,
            label: self.label,
            started_at: self.started_at,
            committed_at: worldos_kernel::now_ms(),
            commands: self.commands,
            ops: self.ops,
            undone: false,
        }
    }
}

/// An immutable committed transaction — the unit of history, undo and diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub id: TransactionId,
    pub index: u64,
    pub actor: ActorId,
    pub label: String,
    pub started_at: TimestampMs,
    pub committed_at: TimestampMs,
    pub commands: Vec<CommandRecord>,
    pub ops: Vec<StateOp>,
    #[serde(default)]
    pub undone: bool,
}
