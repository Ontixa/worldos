//! Relation commands: typed edges between objects.

use super::object::resolve_object;
use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use worldos_kernel::ids::RelationId;
use worldos_kernel::model::Relation;

pub struct RelationAdd;

impl CommandHandler for RelationAdd {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "relation.add",
            "relation",
            "Add a typed relation between two objects (from/to accept id or name)",
            props::object(
                &["type", "from", "to"],
                json!({
                    "type": {"type": "string", "description": "e.g. core:contains, core:depends-on"},
                    "from": {"type": "string"},
                    "to": {"type": "string"},
                    "properties": {"type": "object"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let ty = input["type"].as_str().unwrap().to_string();
        let from = resolve_endpoint(ctx, &input["from"])?;
        let to = resolve_endpoint(ctx, &input["to"])?;
        let mut rel = Relation::new(ty, from, to, &ctx.actor.id);
        if let Some(props) = input.get("properties").and_then(|p| p.as_object()) {
            rel.properties = props.clone().into_iter().collect();
        }
        let id = ctx.put_relation(rel)?;
        Ok(json!({"id": id.to_string()}))
    }
}

fn resolve_endpoint(
    ctx: &CommandContext,
    v: &Value,
) -> Result<worldos_kernel::ids::ObjectId, CommandError> {
    let s = v
        .as_str()
        .ok_or_else(|| CommandError::Failed("endpoint must be string".into()))?;
    if s.parse::<worldos_kernel::ids::ObjectId>().is_ok() {
        resolve_object(ctx, &json!({"id": s}))
    } else {
        resolve_object(ctx, &json!({"name": s}))
    }
}

pub struct RelationRemove;

impl CommandHandler for RelationRemove {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "relation.remove",
            "relation",
            "Remove a relation by id, or by (type, from, to)",
            props::object(
                &[],
                json!({
                    "id": {"type": "string"},
                    "type": {"type": "string"},
                    "from": {"type": "string"},
                    "to": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        if let Some(id) = input.get("id").and_then(|v| v.as_str()) {
            let rid: RelationId = id
                .parse()
                .map_err(|_| CommandError::Failed(format!("malformed relation id `{id}`")))?;
            ctx.remove_relation(rid)?;
            return Ok(json!({"removed": id}));
        }
        let ty = input["type"].as_str().unwrap_or("");
        let from = resolve_endpoint(ctx, &input["from"])?;
        let to = resolve_endpoint(ctx, &input["to"])?;
        let rid = ctx
            .project
            .relations
            .values()
            .find(|r| r.type_id == ty && r.from == from && r.to == to)
            .map(|r| r.id)
            .ok_or_else(|| CommandError::Failed("relation not found".into()))?;
        ctx.remove_relation(rid)?;
        Ok(json!({"removed": rid.to_string()}))
    }
}
