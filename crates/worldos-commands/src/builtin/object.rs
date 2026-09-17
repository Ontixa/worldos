//! Object lifecycle commands.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use worldos_kernel::error::KernelError;
use worldos_kernel::ids::ObjectId;
use worldos_kernel::known;
use worldos_kernel::model::{Component, Object, Relation};

/// Resolve `{id}` or `{name}` input to an object id.
pub fn resolve_object(ctx: &CommandContext, input: &Value) -> Result<ObjectId, CommandError> {
    if let Some(id) = input.get("id").and_then(|v| v.as_str()) {
        let oid: ObjectId = id
            .parse()
            .map_err(|_| CommandError::Failed(format!("malformed id `{id}`")))?;
        if ctx.project.get(oid).is_none() {
            return Err(KernelError::ObjectNotFound(id.into()).into());
        }
        return Ok(oid);
    }
    if let Some(name) = input.get("name").and_then(|v| v.as_str()) {
        return ctx
            .project
            .find_by_name(name)
            .map(|o| o.id)
            .ok_or_else(|| KernelError::ObjectNotFound(name.into()).into());
    }
    Err(CommandError::Failed("provide `id` or `name`".into()))
}

pub struct ObjectCreate;

impl CommandHandler for ObjectCreate {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.create",
            "object",
            "Create an object of a semantic type with optional components, tags and parent",
            props::object(
                &["type", "name"],
                json!({
                    "type": {"type": "string", "description": "semantic type id e.g. core:note"},
                    "name": {"type": "string"},
                    "components": {"type": "object", "description": "map of component type -> data"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "parent": {"type": "string", "description": "parent object id or name"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let type_id = input["type"].as_str().unwrap().to_string();
        let name = input["name"].as_str().unwrap().to_string();
        let mut obj = Object::new(type_id.as_str(), name, &ctx.actor.id);
        if let Some(comps) = input.get("components").and_then(|c| c.as_object()) {
            for (ctype, data) in comps {
                obj.set_component(Component::new(ctype.clone(), data.clone()));
            }
        }
        if let Some(tags) = input.get("tags").and_then(|t| t.as_array()) {
            obj.tags = tags
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
        }
        let id = ctx.insert_object(obj)?;
        if let Some(parent) = input.get("parent").and_then(|p| p.as_str()) {
            let pid = resolve_object(ctx, &json!({"id": parent}))
                .or_else(|_| resolve_object(ctx, &json!({"name": parent})))?;
            ctx.put_relation(Relation::new(known::rel::CONTAINS, pid, id, &ctx.actor.id))?;
        }
        Ok(json!({"id": id.to_string()}))
    }
}

pub struct ObjectDelete;

impl CommandHandler for ObjectDelete {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.delete",
            "object",
            "Delete an object (and contained subtree when cascade=true)",
            props::object(
                &[],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "cascade": {"type": "boolean", "description": "delete contained children too"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let cascade = input
            .get("cascade")
            .and_then(|c| c.as_bool())
            .unwrap_or(false);
        let targets = if cascade {
            ctx.project.collect_subtree(id)
        } else {
            vec![id]
        };
        let mut deleted = Vec::new();
        // delete children first so undo restores parents last
        for oid in targets.into_iter().rev() {
            if let Ok(obj) = ctx.remove_object(oid) {
                deleted.push(json!({"id": obj.id.to_string(), "name": obj.name}));
            }
        }
        Ok(json!({"deleted": deleted}))
    }
}

pub struct ObjectRename;

impl CommandHandler for ObjectRename {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.rename",
            "object",
            "Rename an object",
            props::object(
                &["name"],
                json!({
                    "id": {"type": "string"}, "object": {"type": "string", "description": "current name or id"},
                    "name": {"type": "string", "description": "new name"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let target = input
            .get("object")
            .or_else(|| input.get("id"))
            .and_then(|v| v.as_str());
        let lookup = match target {
            Some(t) if t.parse::<ObjectId>().is_ok() => json!({"id": t}),
            Some(t) => json!({"name": t}),
            None => input.clone(),
        };
        let id = resolve_object(ctx, &lookup)?;
        let new_name = input["name"].as_str().unwrap().to_string();
        ctx.update_object(id, |o| o.name = new_name.clone())?;
        Ok(json!({"id": id.to_string(), "name": new_name}))
    }
}

pub struct ObjectSetProperty;

impl CommandHandler for ObjectSetProperty {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.set_property",
            "object",
            "Set a nested property inside a component's data (creates the component if absent)",
            props::object(
                &["component", "path", "value"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "component": {"type": "string"},
                    "path": {"type": "string", "description": "dot path e.g. size.x"},
                    "value": {}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let comp = input["component"].as_str().unwrap().to_string();
        let path = input["path"].as_str().unwrap().to_string();
        let value = input["value"].clone();
        ctx.update_object(id, |o| {
            let entry = o
                .components
                .entry(comp.clone())
                .or_insert_with(|| Component::new(comp.clone(), json!({})));
            set_path(&mut entry.data, &path, value.clone());
        })?;
        Ok(json!({"id": id.to_string(), "component": comp, "path": path}))
    }
}

/// Set `root.a.b.c = value`, creating objects along the way.
pub fn set_path(root: &mut Value, path: &str, value: Value) {
    let mut cur = root;
    let mut segs = path.split('.').peekable();
    while let Some(seg) = segs.next() {
        if segs.peek().is_none() {
            if let Value::Object(m) = cur {
                m.insert(seg.to_string(), value);
            }
            return;
        }
        if !cur.get(seg).map(|v| v.is_object()).unwrap_or(false)
            && let Value::Object(m) = cur
        {
            m.insert(seg.to_string(), json!({}));
        }
        cur = cur.get_mut(seg).unwrap();
    }
}

pub struct ObjectSetComponent;

impl CommandHandler for ObjectSetComponent {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.set_component",
            "object",
            "Attach or replace a component on an object",
            props::object(
                &["component", "data"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "component": {"type": "string"},
                    "version": {"type": "integer"},
                    "data": {"type": "object"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let ctype = input["component"].as_str().unwrap().to_string();
        let data = input["data"].clone();
        let version = input.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        ctx.update_object(id, |o| {
            let mut c = Component::new(ctype.clone(), data.clone());
            c.version = version;
            o.set_component(c);
        })?;
        Ok(json!({"id": id.to_string(), "component": ctype}))
    }
}

pub struct ObjectRemoveComponent;

impl CommandHandler for ObjectRemoveComponent {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.remove_component",
            "object",
            "Remove a component from an object",
            props::object(
                &["component"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "component": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let ctype = input["component"].as_str().unwrap().to_string();
        ctx.update_object(id, |o| {
            o.components.remove(&ctype);
        })?;
        Ok(json!({"id": id.to_string(), "removed": ctype}))
    }
}

pub struct ObjectAddTag;

impl CommandHandler for ObjectAddTag {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.add_tag",
            "object",
            "Add a tag to an object",
            props::object(
                &["tag"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "tag": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let tag = input["tag"].as_str().unwrap().to_string();
        ctx.update_object(id, |o| {
            if !o.tags.contains(&tag) {
                o.tags.push(tag.clone());
            }
        })?;
        Ok(json!({"id": id.to_string(), "tag": tag}))
    }
}

pub struct ObjectRemoveTag;

impl CommandHandler for ObjectRemoveTag {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "object.remove_tag",
            "object",
            "Remove a tag from an object",
            props::object(
                &["tag"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "tag": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = resolve_object(ctx, input)?;
        let tag = input["tag"].as_str().unwrap().to_string();
        ctx.update_object(id, |o| o.tags.retain(|t| *t != tag))?;
        Ok(json!({"id": id.to_string(), "removed": tag}))
    }
}
