//! Code artifact commands.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{props, CommandSchema};
use serde_json::{json, Value};
use worldos_kernel::known::{components, types};

pub struct CodeCreateFile;

impl CommandHandler for CodeCreateFile {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "code.create_file",
            "code",
            "Create a source-file object with language and source text",
            props::object(
                &["name"],
                json!({
                    "name": {"type": "string"},
                    "language": {"type": "string"},
                    "source": {"type": "string"},
                    "path": {"type": "string", "description": "logical path e.g. src/main.rs"},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let source = input.get("source").and_then(|s| s.as_str()).unwrap_or("");
        let language = input.get("language").and_then(|l| l.as_str()).unwrap_or("text");
        let path = input.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let mut args = json!({
            "type": types::CODE_FILE,
            "name": input["name"],
            "components": {
                components::SOURCE: {"language": language, "source": source, "path": path}
            },
        });
        if let Some(p) = input.get("parent") {
            args["parent"] = p.clone();
        }
        ctx.run_sub("object.create", args)
    }
}

pub struct CodeSetSource;

impl CommandHandler for CodeSetSource {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "code.set_source",
            "code",
            "Replace the source text of a code file object",
            props::object(
                &["source"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "source": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = super::object::resolve_object(ctx, input)?;
        let source = input["source"].as_str().unwrap_or("").to_string();
        ctx.update_object(id, |o| {
            let entry = o
                .components
                .entry(components::SOURCE.to_string())
                .or_insert_with(|| {
                    worldos_kernel::Component::new(
                        components::SOURCE,
                        json!({"language": "text", "path": ""}),
                    )
                });
            entry.data["source"] = json!(source);
        })?;
        Ok(json!({"id": id.to_string()}))
    }
}
