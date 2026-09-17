//! Semantic project diff: compares two snapshots at the object/component
//! level — "motor.thickness 3.0 → 2.5", not "file changed".

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use worldos_kernel::project::Project;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffEntry {
    ObjectAdded {
        id: String,
        name: String,
        type_id: String,
    },
    ObjectRemoved {
        id: String,
        name: String,
        type_id: String,
    },
    ObjectChanged {
        id: String,
        name: String,
        changes: Vec<FieldChange>,
    },
    RelationAdded {
        id: String,
        type_id: String,
        from: String,
        to: String,
    },
    RelationRemoved {
        id: String,
        type_id: String,
        from: String,
        to: String,
    },
    ProjectMetaChanged {
        key: String,
        before: Option<Value>,
        after: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub path: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// Diff two project states. `a` = older, `b` = newer.
pub fn diff_projects(a: &Project, b: &Project) -> Vec<DiffEntry> {
    let mut out = Vec::new();
    if a.name != b.name {
        out.push(DiffEntry::ProjectMetaChanged {
            key: "name".into(),
            before: Some(Value::String(a.name.clone())),
            after: Some(Value::String(b.name.clone())),
        });
    }
    for (id, obj) in &b.objects {
        match a.objects.get(id) {
            None => out.push(DiffEntry::ObjectAdded {
                id: id.to_string(),
                name: obj.name.clone(),
                type_id: obj.type_id.0.clone(),
            }),
            Some(prev) => {
                let changes = object_changes(prev, obj);
                if !changes.is_empty() {
                    out.push(DiffEntry::ObjectChanged {
                        id: id.to_string(),
                        name: obj.name.clone(),
                        changes,
                    });
                }
            }
        }
    }
    for (id, obj) in &a.objects {
        if !b.objects.contains_key(id) {
            out.push(DiffEntry::ObjectRemoved {
                id: id.to_string(),
                name: obj.name.clone(),
                type_id: obj.type_id.0.clone(),
            });
        }
    }
    for (id, rel) in &b.relations {
        if !a.relations.contains_key(id) {
            out.push(DiffEntry::RelationAdded {
                id: id.to_string(),
                type_id: rel.type_id.clone(),
                from: rel.from.to_string(),
                to: rel.to.to_string(),
            });
        }
    }
    for (id, rel) in &a.relations {
        if !b.relations.contains_key(id) {
            out.push(DiffEntry::RelationRemoved {
                id: id.to_string(),
                type_id: rel.type_id.clone(),
                from: rel.from.to_string(),
                to: rel.to.to_string(),
            });
        }
    }
    out
}

fn object_changes(a: &worldos_kernel::Object, b: &worldos_kernel::Object) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    if a.name != b.name {
        changes.push(FieldChange {
            path: "name".into(),
            before: Some(Value::String(a.name.clone())),
            after: Some(Value::String(b.name.clone())),
        });
    }
    if a.tags != b.tags {
        changes.push(FieldChange {
            path: "tags".into(),
            before: Some(serde_json::to_value(&a.tags).unwrap_or_default()),
            after: Some(serde_json::to_value(&b.tags).unwrap_or_default()),
        });
    }
    // flatten component data leaf values → path map
    let mut fa = BTreeMap::new();
    let mut fb = BTreeMap::new();
    for (ctype, comp) in &a.components {
        flatten(&comp.data, &format!("components.{ctype}"), &mut fa);
    }
    for (ctype, comp) in &b.components {
        flatten(&comp.data, &format!("components.{ctype}"), &mut fb);
    }
    for (k, av) in &fa {
        match fb.get(k) {
            None => changes.push(FieldChange {
                path: k.clone(),
                before: Some(av.clone()),
                after: None,
            }),
            Some(bv) if bv != av => changes.push(FieldChange {
                path: k.clone(),
                before: Some(av.clone()),
                after: Some(bv.clone()),
            }),
            _ => {}
        }
    }
    for (k, bv) in &fb {
        if !fa.contains_key(k) {
            changes.push(FieldChange {
                path: k.clone(),
                before: None,
                after: Some(bv.clone()),
            });
        }
    }
    changes
}

/// Flatten JSON to leaf `path -> value` entries (objects recurse).
fn flatten(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, sub) in m {
                flatten(sub, &format!("{prefix}.{k}"), out);
            }
        }
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}
