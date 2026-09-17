//! Core object model: objects, components, relations.
//!
//! Everything meaningful is an object. Behavior and domain meaning come
//! from composable, schema-versioned components — not inheritance.

use crate::ids::{ActorId, ObjectId, RelationId, TypeId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Milliseconds since Unix epoch.
pub type TimestampMs = i64;

pub fn now_ms() -> TimestampMs {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}

/// A schema-versioned component payload attached to an object.
/// `data` is JSON validated against the component type's schema when known.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub type_id: String,
    pub version: u32,
    pub data: Value,
}

impl Component {
    pub fn new(type_id: impl Into<String>, data: Value) -> Self {
        Self { type_id: type_id.into(), version: 1, data }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub created_at: TimestampMs,
    pub created_by: ActorId,
    pub updated_at: TimestampMs,
    pub updated_by: ActorId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
    #[serde(default)]
    pub revision: u64,
}

impl ObjectMeta {
    pub fn new(actor: &ActorId) -> Self {
        let now = now_ms();
        Self {
            created_at: now,
            created_by: actor.clone(),
            updated_at: now,
            updated_by: actor.clone(),
            provenance: None,
            revision: 0,
        }
    }
    pub fn touch(&mut self, actor: &ActorId) {
        self.updated_at = now_ms();
        self.updated_by = actor.clone();
        self.revision += 1;
    }
}

/// A node in the universal project graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub type_id: TypeId,
    pub name: String,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub meta: ObjectMeta,
}

impl Object {
    pub fn new(type_id: impl Into<TypeId>, name: impl Into<String>, actor: &ActorId) -> Self {
        Self {
            id: ObjectId::new(),
            type_id: type_id.into(),
            name: name.into(),
            components: BTreeMap::new(),
            tags: Vec::new(),
            meta: ObjectMeta::new(actor),
        }
    }
    pub fn component(&self, type_id: &str) -> Option<&Component> {
        self.components.get(type_id)
    }
    pub fn component_data(&self, type_id: &str) -> Option<&Value> {
        self.components.get(type_id).map(|c| &c.data)
    }
    pub fn set_component(&mut self, component: Component) {
        self.components.insert(component.type_id.clone(), component);
    }
}

/// A directed, typed edge between two objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: RelationId,
    pub type_id: String,
    pub from: ObjectId,
    pub to: ObjectId,
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
    #[serde(default)]
    pub meta: Option<ObjectMeta>,
}

impl Relation {
    pub fn new(
        type_id: impl Into<String>,
        from: ObjectId,
        to: ObjectId,
        actor: &ActorId,
    ) -> Self {
        Self {
            id: RelationId::new(),
            type_id: type_id.into(),
            from,
            to,
            properties: BTreeMap::new(),
            meta: Some(ObjectMeta::new(actor)),
        }
    }
}
