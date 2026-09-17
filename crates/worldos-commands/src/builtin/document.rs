//! Document (note) commands.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{props, CommandSchema};
use serde_json::{json, Value};
use worldos_kernel::known::{components, types};

pub struct DocumentCreate;

impl CommandHandler for DocumentCreate {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "document.create",
            "document",
            "Create a note/document object with text content",
            props::object(
                &["name"],
                json!({
                    "name": {"type": "string"},
                    "text": {"type": "string"},
                    "format": {"type": "string", "enum": ["markdown", "plain"]},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let text = input.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let format = input.get("format").and_then(|f| f.as_str()).unwrap_or("markdown");
        let mut args = json!({
            "type": types::NOTE,
            "name": input["name"],
            "components": { components::TEXT: {"text": text, "format": format} },
        });
        if let Some(p) = input.get("parent") {
            args["parent"] = p.clone();
        }
        ctx.run_sub("object.create", args)
    }
}

fn set_text(ctx: &mut CommandContext, input: &Value, append: bool) -> Result<Value, CommandError> {
    let id = super::object::resolve_object(ctx, input)?;
    let text = input["text"].as_str().unwrap_or("");
    ctx.update_object(id, |o| {
        let entry = o
            .components
            .entry(components::TEXT.to_string())
            .or_insert_with(|| {
                worldos_kernel::Component::new(components::TEXT, json!({"format": "markdown"}))
            });
        let cur = entry.data.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let next = if append { format!("{cur}{text}") } else { text.to_string() };
        entry.data["text"] = json!(next);
    })?;
    Ok(json!({"id": id.to_string()}))
}

pub struct DocumentSetText;

impl CommandHandler for DocumentSetText {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "document.set_text",
            "document",
            "Replace the text of a note/document",
            props::object(
                &["text"],
                json!({"id": {"type": "string"}, "name": {"type": "string"}, "text": {"type": "string"}}),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        set_text(ctx, input, false)
    }
}

pub struct DocumentAppendText;

impl CommandHandler for DocumentAppendText {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "document.append_text",
            "document",
            "Append text to a note/document",
            props::object(
                &["text"],
                json!({"id": {"type": "string"}, "name": {"type": "string"}, "text": {"type": "string"}}),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        set_text(ctx, input, true)
    }
}
