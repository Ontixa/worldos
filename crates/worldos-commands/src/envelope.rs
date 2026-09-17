//! Command envelopes and records: serializable descriptions of intent
//! and of what actually happened.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worldos_kernel::ids::{CommandId, TransactionId};
use worldos_kernel::{ActorId, TimestampMs};

/// The serializable request that produced (or will produce) a mutation.
/// Same envelope shape is used by GUI, CLI, SDK, MCP and agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: CommandId,
    pub command_type: String,
    pub actor: ActorId,
    pub inputs: Value,
    pub created_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<TransactionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

impl CommandEnvelope {
    pub fn new(command_type: impl Into<String>, actor: ActorId, inputs: Value) -> Self {
        Self {
            id: CommandId::new(),
            command_type: command_type.into(),
            actor,
            inputs,
            created_at: worldos_kernel::now_ms(),
            transaction_id: None,
            parent_action: None,
            provenance: None,
        }
    }
}

/// What the engine returns after executing one command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub command_id: CommandId,
    pub transaction_id: TransactionId,
    pub output: Value,
}

/// Persistent record of one executed command inside a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub envelope: CommandEnvelope,
    pub ok: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
