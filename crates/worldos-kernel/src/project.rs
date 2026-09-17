//! The Universal Project Graph (UPG): in-memory project state.
//!
//! Holds all objects and relations with secondary indexes for efficient
//! traversal. All mutations are performed by applying `StateOp`s so that
//! history, undo and diff share one code path.

use crate::delta::StateOp;
use crate::error::KernelError;
use crate::ids::{ObjectId, ProjectId, RelationId};
use crate::model::{Object, Relation, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Current on-disk/in-memory schema version of the project model.
pub const PROJECT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub schema_version: u32,
    pub created_at: i64,
    #[serde(default)]
    pub objects: HashMap<ObjectId, Object>,
    #[serde(default)]
    pub relations: HashMap<RelationId, Relation>,
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ProjectId::new(),
            name: name.into(),
            schema_version: PROJECT_SCHEMA_VERSION,
            created_at: now_ms(),
            objects: HashMap::new(),
            relations: HashMap::new(),
            settings: BTreeMap::new(),
        }
    }

    // ----- queries -------------------------------------------------------

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id)
    }
    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.objects.get_mut(&id)
    }
    pub fn find_by_name(&self, name: &str) -> Option<&Object> {
        self.objects.values().find(|o| o.name == name)
    }
    pub fn objects_of_type(&self, type_id: &str) -> impl Iterator<Item = &Object> {
        self.objects
            .values()
            .filter(move |o| o.type_id.0 == type_id)
    }
    pub fn relations_of(&self, id: ObjectId) -> impl Iterator<Item = &Relation> {
        self.relations
            .values()
            .filter(move |r| r.from == id || r.to == id)
    }
    pub fn relations_from(&self, id: ObjectId) -> impl Iterator<Item = &Relation> {
        self.relations.values().filter(move |r| r.from == id)
    }
    pub fn relations_to(&self, id: ObjectId) -> impl Iterator<Item = &Relation> {
        self.relations.values().filter(move |r| r.to == id)
    }
    /// Children via the `core:contains` containment relation.
    pub fn children(&self, id: ObjectId) -> Vec<&Object> {
        self.relations_from(id)
            .filter(|r| r.type_id == crate::known::rel::CONTAINS)
            .filter_map(|r| self.get(r.to))
            .collect()
    }
    pub fn parent(&self, id: ObjectId) -> Option<&Object> {
        self.relations_to(id)
            .find(|r| r.type_id == crate::known::rel::CONTAINS)
            .and_then(|r| self.get(r.from))
    }
    /// Root objects: not contained by another object.
    pub fn roots(&self) -> Vec<&Object> {
        let contained: HashSet<ObjectId> = self
            .relations
            .values()
            .filter(|r| r.type_id == crate::known::rel::CONTAINS)
            .map(|r| r.to)
            .collect();
        self.objects
            .values()
            .filter(|o| !contained.contains(&o.id))
            .collect()
    }
    pub fn sorted_objects(&self) -> Vec<&Object> {
        let mut v: Vec<&Object> = self.objects.values().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        v
    }

    // ----- ops -----------------------------------------------------------

    /// Apply one side of a `StateOp`. `forward` applies `after`, `!forward`
    /// applies `before`. Returns the inverse op so callers can record it.
    pub fn apply(&mut self, op: &StateOp, forward: bool) -> Result<(), KernelError> {
        match op {
            StateOp::SetObject { before, after } => {
                let target = if forward { after } else { before };
                match target {
                    Some(obj) => {
                        self.objects.insert(obj.id, (**obj).clone());
                    }
                    None => {
                        // Target is None: remove the object carried by the other side.
                        let other = if forward { before } else { after };
                        if let Some(o) = other {
                            self.objects.remove(&o.id);
                        }
                    }
                }
            }
            StateOp::SetRelation { before, after } => {
                let target = if forward { after } else { before };
                match target {
                    Some(rel) => {
                        self.relations.insert(rel.id, (**rel).clone());
                    }
                    None => {
                        let other = if forward { before } else { after };
                        if let Some(r) = other {
                            self.relations.remove(&r.id);
                        }
                    }
                }
            }
            StateOp::SetProjectMeta { key, before, after } => {
                let target = if forward { after } else { before };
                match target {
                    Some(v) => {
                        self.settings.insert(key.clone(), v.clone());
                    }
                    None => {
                        self.settings.remove(key);
                    }
                }
            }
        }
        Ok(())
    }

    /// Referential integrity check used by validators.
    pub fn dangling_relations(&self) -> Vec<&Relation> {
        self.relations
            .values()
            .filter(|r| !self.objects.contains_key(&r.from) || !self.objects.contains_key(&r.to))
            .collect()
    }

    /// Delete object plus all relations that reference it.
    /// Caller is responsible for recording the inverse ops.
    pub fn collect_subtree(&self, root: ObjectId) -> Vec<ObjectId> {
        let mut out = vec![root];
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            for rel in self.relations_from(id) {
                if rel.type_id == crate::known::rel::CONTAINS {
                    out.push(rel.to);
                    stack.push(rel.to);
                }
            }
        }
        out
    }
}
