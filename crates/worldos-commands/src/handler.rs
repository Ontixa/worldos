//! Command handler trait, execution context, and the registry.

use crate::error::CommandError;
use crate::schema::CommandSchema;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use worldos_kernel::actor::Actor;
use worldos_kernel::delta::StateOp;
use worldos_kernel::error::KernelError;
use worldos_kernel::ids::{ObjectId, RelationId};
use worldos_kernel::model::{Object, Relation};
use worldos_kernel::project::Project;

/// Mutable view of the world a command may touch. All mutations MUST go
/// through these helpers so undo ops are recorded automatically.
pub struct CommandContext<'a> {
    pub project: &'a mut Project,
    pub ops: &'a mut Vec<StateOp>,
    pub actor: &'a Actor,
    pub registry: &'a CommandRegistry,
}

impl<'a> CommandContext<'a> {
    /// Insert a new object, recording the create op.
    pub fn insert_object(&mut self, obj: Object) -> Result<ObjectId, KernelError> {
        let op = StateOp::object_created(obj);
        self.project.apply(&op, true)?;
        let id = match &op {
            StateOp::SetObject { after, .. } => after.as_ref().unwrap().id,
            _ => unreachable!(),
        };
        self.ops.push(op);
        Ok(id)
    }

    /// Mutate an existing object; the before-state is snapshotted for undo.
    pub fn update_object<R>(
        &mut self,
        id: ObjectId,
        f: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, KernelError> {
        let before = self
            .project
            .get(id)
            .cloned()
            .ok_or_else(|| KernelError::ObjectNotFound(id.to_string()))?;
        let mut after = before.clone();
        let r = f(&mut after);
        after.meta.touch(&self.actor.id);
        let op = StateOp::object_updated(before, after);
        self.project.apply(&op, true)?;
        self.ops.push(op);
        Ok(r)
    }

    /// Remove an object plus every relation touching it.
    pub fn remove_object(&mut self, id: ObjectId) -> Result<Object, KernelError> {
        let obj = self
            .project
            .get(id)
            .cloned()
            .ok_or_else(|| KernelError::ObjectNotFound(id.to_string()))?;
        // remove attached relations first (order matters for undo)
        let rel_ids: Vec<RelationId> =
            self.project.relations_of(id).map(|r| r.id).collect();
        for rid in rel_ids {
            self.remove_relation(rid)?;
        }
        let op = StateOp::object_deleted(obj.clone());
        self.project.apply(&op, true)?;
        self.ops.push(op);
        Ok(obj)
    }

    pub fn put_relation(&mut self, rel: Relation) -> Result<RelationId, KernelError> {
        if self.project.get(rel.from).is_none() || self.project.get(rel.to).is_none() {
            return Err(KernelError::DanglingRelation(rel.id.to_string()));
        }
        let id = rel.id;
        let op = StateOp::relation_created(rel);
        self.project.apply(&op, true)?;
        self.ops.push(op);
        Ok(id)
    }

    pub fn remove_relation(&mut self, id: RelationId) -> Result<(), KernelError> {
        let rel = self
            .project
            .relations
            .get(&id)
            .cloned()
            .ok_or_else(|| KernelError::RelationNotFound(id.to_string()))?;
        let op = StateOp::relation_deleted(rel);
        self.project.apply(&op, true)?;
        self.ops.push(op);
        Ok(())
    }

    /// Execute another command inside this same transaction — composite
    /// commands stay one undoable unit.
    pub fn run_sub(&mut self, command_type: &str, input: Value) -> Result<Value, CommandError> {
        let handler = self
            .registry
            .handler(command_type)
            .ok_or_else(|| CommandError::Unknown(command_type.into()))?;
        let schema = handler.schema();
        let errors = worldos_kernel::schema::validate(&schema.input_schema, &input, "$");
        if !errors.is_empty() {
            return Err(CommandError::Validation {
                command: command_type.into(),
                errors,
            });
        }
        let mut sub = CommandContext {
            project: self.project,
            ops: self.ops,
            actor: self.actor,
            registry: self.registry,
        };
        handler.execute(&mut sub, &input)
    }
}

/// A registered command implementation.
pub trait CommandHandler: Send + Sync {
    fn schema(&self) -> CommandSchema;
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError>;
}

/// Maps `command_type` strings to handlers. Plugins register more.
#[derive(Default)]
pub struct CommandRegistry {
    handlers: HashMap<String, Arc<dyn CommandHandler>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, handler: Arc<dyn CommandHandler>) {
        let ty = handler.schema().command_type.clone();
        self.handlers.insert(ty, handler);
    }
    pub fn handler(&self, command_type: &str) -> Option<Arc<dyn CommandHandler>> {
        self.handlers.get(command_type).cloned()
    }
    pub fn schemas(&self) -> Vec<CommandSchema> {
        let mut v: Vec<_> = self.handlers.values().map(|h| h.schema()).collect();
        v.sort_by(|a, b| a.command_type.cmp(&b.command_type));
        v
    }
    pub fn contains(&self, command_type: &str) -> bool {
        self.handlers.contains_key(command_type)
    }
}
