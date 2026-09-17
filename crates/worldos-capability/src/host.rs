//! `CapabilityHost`: the narrow bridge between capabilities and the
//! engine. Capabilities can only touch the world through this trait —
//! which means every mutation they cause still flows through commands.

use crate::error::CapabilityError;
use serde_json::Value;
use worldos_commands::CommandSchema;
use worldos_kernel::actor::Actor;
use worldos_kernel::project::Project;
use worldos_kernel::validation::ValidationReport;

pub trait CapabilityHost {
    fn project(&self) -> &Project;
    fn actor(&self) -> &Actor;

    /// Execute a command as this host's actor. Joins the currently open
    /// transaction when one exists, otherwise runs as its own transaction.
    fn run_command(&mut self, command_type: &str, input: Value)
        -> Result<Value, CapabilityError>;

    /// Execute a command as a specific actor (e.g. an agent identity).
    fn run_command_as(
        &mut self,
        actor: &Actor,
        command_type: &str,
        input: Value,
    ) -> Result<Value, CapabilityError>;

    /// Transaction control for capability orchestration (agents group
    /// their actions into one undoable transaction).
    fn begin_transaction(&mut self, label: &str) -> Result<(), CapabilityError>;
    /// Begin a transaction attributed to a specific actor (e.g. an agent).
    fn begin_transaction_as(
        &mut self,
        actor: &Actor,
        label: &str,
    ) -> Result<(), CapabilityError>;
    fn commit_transaction(&mut self) -> Result<(), CapabilityError>;
    fn rollback_transaction(&mut self) -> Result<(), CapabilityError>;
    fn in_transaction(&self) -> bool;

    /// Machine-readable schemas of every registered command — planners,
    /// MCP tool discovery and the command palette all read this.
    fn command_schemas(&self) -> Vec<CommandSchema>;

    /// Run all registered validators and return a structured report.
    fn validate(&self) -> Result<ValidationReport, CapabilityError>;

    /// Resolve an object by id-or-name helper string.
    fn resolve_object(&self, id_or_name: &str) -> Option<worldos_kernel::ObjectId> {
        if let Ok(id) = id_or_name.parse() {
            if self.project().get(id).is_some() {
                return Some(id);
            }
        }
        self.project().find_by_name(id_or_name).map(|o| o.id)
    }
}
