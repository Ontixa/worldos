//! Agent run reporting: what was asked, what was done, what was verified.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worldos_kernel::ids::{AgentRunId, TransactionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Planned,
    Succeeded,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub index: usize,
    pub command: String,
    pub input: Value,
    pub note: String,
    pub ok: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReport {
    pub run_id: AgentRunId,
    pub agent: String,
    pub goal: String,
    pub status: RunStatus,
    pub steps: Vec<StepRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<TransactionId>,
    #[serde(default)]
    pub created_objects: Vec<String>,
    pub verification: Vec<String>,
    pub summary: String,
}
