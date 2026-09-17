//! Engine events: structured notifications emitted for every meaningful
//! occurrence. UI, agents and logs subscribe to the same stream.

use crate::ids::{ActorId, CommandId, ObjectId, TransactionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    ObjectCreated { object_id: ObjectId, type_id: String, name: String },
    ObjectUpdated { object_id: ObjectId },
    ObjectDeleted { object_id: ObjectId, name: String },
    CommandExecuted { command_id: CommandId, command_type: String, actor: ActorId },
    TransactionCommitted {
        transaction_id: TransactionId,
        actor: ActorId,
        label: String,
        command_count: usize,
        affected: Vec<ObjectId>,
    },
    TransactionUndone { transaction_id: TransactionId },
    TransactionRedone { transaction_id: TransactionId },
    ValidationCompleted { errors: usize, warnings: usize },
    AgentRunStarted { run_id: String, goal: String },
    AgentRunFinished { run_id: String, status: String, summary: String },
    ProjectSaved { path: String },
    ProjectLoaded { path: String },
    /// Generic structured event for extensions.
    Custom { topic: String, data: Value },
}

impl EngineEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ObjectCreated { .. } => "object_created",
            Self::ObjectUpdated { .. } => "object_updated",
            Self::ObjectDeleted { .. } => "object_deleted",
            Self::CommandExecuted { .. } => "command_executed",
            Self::TransactionCommitted { .. } => "transaction_committed",
            Self::TransactionUndone { .. } => "transaction_undone",
            Self::TransactionRedone { .. } => "transaction_redone",
            Self::ValidationCompleted { .. } => "validation_completed",
            Self::AgentRunStarted { .. } => "agent_run_started",
            Self::AgentRunFinished { .. } => "agent_run_finished",
            Self::ProjectSaved { .. } => "project_saved",
            Self::ProjectLoaded { .. } => "project_loaded",
            Self::Custom { .. } => "custom",
        }
    }
}
