//! Project-level meta commands.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use worldos_kernel::delta::StateOp;

pub struct ProjectRename;

impl CommandHandler for ProjectRename {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "project.rename",
            "project",
            "Rename the project",
            props::object(&["name"], json!({"name": {"type": "string"}})),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let name = input["name"].as_str().unwrap().to_string();
        let before = json!(ctx.project.name);
        ctx.project.name = name.clone();
        ctx.ops.push(StateOp::SetProjectMeta {
            key: "__name".into(),
            before: Some(before),
            after: Some(json!(name)),
        });
        Ok(json!({"name": ctx.project.name}))
    }
}

pub struct ProjectSetMeta;

impl CommandHandler for ProjectSetMeta {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "project.set_meta",
            "project",
            "Set a project settings key",
            props::object(
                &["key", "value"],
                json!({"key": {"type": "string"}, "value": {}}),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let key = input["key"].as_str().unwrap().to_string();
        let value = input["value"].clone();
        let before = ctx.project.settings.get(&key).cloned();
        let op = StateOp::SetProjectMeta {
            key: key.clone(),
            before,
            after: Some(value),
        };
        ctx.project.apply(&op, true)?;
        ctx.ops.push(op);
        Ok(json!({"key": key}))
    }
}
