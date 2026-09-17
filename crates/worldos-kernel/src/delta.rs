//! State operations: the atomic, invertible unit of change.
//!
//! Every mutation to the project is captured as a `StateOp` recording both
//! the before and after state of the touched entity. Applying `after`
//! performs the change; applying `before` reverts it. This single mechanism
//! powers undo, redo, semantic diff, replay and audit.

use crate::model::{Object, Relation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateOp {
    /// Object created (`before: None`), deleted (`after: None`) or updated.
    SetObject {
        before: Option<Box<Object>>,
        after: Option<Box<Object>>,
    },
    /// Relation created, removed or updated.
    SetRelation {
        before: Option<Box<Relation>>,
        after: Option<Box<Relation>>,
    },
    /// Project-level metadata change (name, settings).
    SetProjectMeta {
        key: String,
        before: Option<Value>,
        after: Option<Value>,
    },
}

impl StateOp {
    pub fn object_created(obj: Object) -> Self {
        Self::SetObject { before: None, after: Some(Box::new(obj)) }
    }
    pub fn object_updated(before: Object, after: Object) -> Self {
        Self::SetObject { before: Some(Box::new(before)), after: Some(Box::new(after)) }
    }
    pub fn object_deleted(obj: Object) -> Self {
        Self::SetObject { before: Some(Box::new(obj)), after: None }
    }
    pub fn relation_created(rel: Relation) -> Self {
        Self::SetRelation { before: None, after: Some(Box::new(rel)) }
    }
    pub fn relation_deleted(rel: Relation) -> Self {
        Self::SetRelation { before: Some(Box::new(rel)), after: None }
    }

    /// Short human/agent readable summary of the change.
    pub fn describe(&self) -> String {
        match self {
            Self::SetObject { before, after } => match (before, after) {
                (None, Some(a)) => format!("create object {} ({})", a.name, a.type_id),
                (Some(b), None) => format!("delete object {} ({})", b.name, b.type_id),
                (Some(b), Some(a)) if b.name != a.name => {
                    format!("rename object {} -> {}", b.name, a.name)
                }
                (Some(b), Some(_)) => format!("update object {}", b.name),
                (None, None) => "noop".into(),
            },
            Self::SetRelation { before, after } => match (before, after) {
                (None, Some(a)) => format!("add relation {} {} -> {}", a.type_id, a.from, a.to),
                (Some(b), None) => format!("remove relation {} {} -> {}", b.type_id, b.from, b.to),
                _ => "update relation".into(),
            },
            Self::SetProjectMeta { key, .. } => format!("set project meta {key}"),
        }
    }
}
