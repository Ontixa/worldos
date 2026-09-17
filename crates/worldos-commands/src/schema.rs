//! Command schema: machine-readable description of a command.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worldos_kernel::known;

/// Declarative description of a command type. Command palette, CLI help,
/// agents and MCP tool discovery all read from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSchema {
    pub command_type: String,
    pub category: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    /// Permission required to execute (checked by the engine).
    pub permission: String,
    /// Whether undo applies (read-only commands may set false).
    pub undoable: bool,
}

impl CommandSchema {
    pub fn write(command_type: &str, category: &str, description: &str, input: Value) -> Self {
        Self {
            command_type: command_type.into(),
            category: category.into(),
            description: description.into(),
            input_schema: input,
            output_schema: None,
            permission: known::permissions::PROJECT_WRITE.into(),
            undoable: true,
        }
    }
    pub fn permission(mut self, perm: &str) -> Self {
        self.permission = perm.into();
        self
    }
}

/// Convenience constructors for common input schema shapes.
pub mod props {
    use serde_json::{json, Value};

    /// `{ "type": "object", "required": [...], "properties": {...} }`
    pub fn object(required: &[&str], properties: Value) -> Value {
        json!({
            "type": "object",
            "required": required,
            "properties": properties,
        })
    }

    /// Standard `id` or `name` object reference input.
    pub fn object_ref() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "object id"},
                "name": {"type": "string", "description": "object name"}
            }
        })
    }
}
