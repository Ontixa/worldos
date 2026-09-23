//! Shared harness for the adversarial property tests (NEXT.md #7).
//!
//! The SUT mints random ULIDs, so generated operations carry pool
//! indices the executor resolves against a creation-ordered model.
//! Every proptest case therefore replays the same *logical* command
//! sequence even though concrete ids differ between runs.

#![allow(dead_code)]

use proptest::prelude::*;
use serde_json::{Value, json};
use worldos_commands::{CommandError, CommandReceipt, History};
use worldos_engine::{Engine, EngineError};
use worldos_kernel::actor::{Actor, ActorKind, PermissionSet};
use worldos_kernel::ids::{ObjectId, RelationId};
use worldos_kernel::project::Project;

/// Relation types exercised by `relation.add`, including `core:contains`
/// so cascade deletes and the containment tree get covered.
pub const REL_POOL: &[&str] = &[
    "core:references",
    "core:depends-on",
    "core:satisfies",
    "core:contains",
];
pub const KIND_POOL: &[&str] = &["cube", "sphere", "cylinder", "cone", "torus", "plane"];
pub const COMP_POOL: &[&str] = &["app:props", "core:cost", "doc:text", "geom:material"];
pub const RM_COMP_POOL: &[&str] = &[
    "app:props",
    "core:cost",
    "doc:text",
    "geom:material",
    "core:transform",
];
/// `project.set_meta` key pool; `__name` is the reserved project-name key.
pub const KEY_POOL: &[&str] = &["k0", "k1", "k2", "k3", "k4", "__name"];
pub const PATH_POOL: &[&str] = &["k", "a.b", "deep.x.y", "n.0"];
/// A well-formed ULID that can never collide with a minted id.
pub const GHOST_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
pub const GHOST_NAME: &str = "ghost-zzz";

/// Object names come from a bounded pool: duplicates are legal (the
/// `UniqueNames` validator only warns), and the executor falls back to
/// id references whenever a name is ambiguous.
pub fn obj_name(ix: u8) -> String {
    if ix % 9 == 8 {
        format!("spec-α{:02}", ix % 7)
    } else {
        format!("n{:02}", ix % 20)
    }
}

fn proj_name(ix: u8) -> String {
    format!("proj-{}", ix % 6)
}

fn leaf(ix: u8) -> Value {
    match ix % 6 {
        0 => json!(ix as i64 - 3),
        1 => json!(ix.is_multiple_of(2)),
        2 => json!(format!("v{ix}")),
        3 => json!(ix as f64 / 7.0),
        4 => json!([ix, ix.wrapping_add(1)]),
        _ => json!({"k": ix}),
    }
}

fn req_expr(ix: u8, dep_name: &str) -> String {
    match ix % 6 {
        0 => format!("exists_named(\"{dep_name}\")"),
        1 => "exists(geom:cube)".into(),
        2 => "count(core:note) >= 0".into(),
        3 => format!("not exists_named(\"{dep_name}\")"),
        4 => "exists(geom:cube) or exists(geom:plane)".into(),
        _ => "count(code:file) < 1000".into(),
    }
}

// ------------------------------------------------------------------ ops

/// One engine command (or deliberately failing call). Indices resolve
/// against the live-object model at execution time.
#[derive(Debug, Clone)]
pub enum Op {
    CreateNote {
        n: u8,
        parent: u8,
    },
    CreateDoc {
        n: u8,
        text: u8,
    },
    CreateCode {
        n: u8,
        text: u8,
    },
    CreatePrimitive {
        n: u8,
        kind: u8,
        size: u8,
        parent: u8,
    },
    CreateRequirement {
        n: u8,
        expr: u8,
        sat: u8,
    },
    CreateDecision {
        n: u8,
        affects: u8,
    },
    Delete {
        pick: u8,
        cascade: bool,
        by_name: bool,
    },
    Rename {
        pick: u8,
        n: u8,
        by_name: bool,
    },
    SetProperty {
        pick: u8,
        path: u8,
        val: u8,
    },
    SetComponent {
        pick: u8,
        comp: u8,
        val: u8,
    },
    RemoveComponent {
        pick: u8,
        comp: u8,
    },
    AddTag {
        pick: u8,
        tag: u8,
    },
    RemoveTag {
        pick: u8,
        tag: u8,
    },
    AddRelation {
        rel: u8,
        from: u8,
        to: u8,
        by_name: bool,
    },
    RemoveRelation {
        pick: u8,
    },
    SetDocText {
        pick: u8,
        text: u8,
        append: bool,
    },
    Transform {
        pick: u8,
        x: i8,
        y: i8,
        z: i8,
    },
    ProjectRename {
        n: u8,
    },
    SetMeta {
        key: u8,
        val: u8,
    },
    Evaluate {
        all: bool,
        pick: u8,
    },
    // --- deliberate failures: must leave zero residue on graph+journal
    BadSchema,
    PermDenied {
        n: u8,
    },
    CreateBadParent {
        n: u8,
    },
    AddRelationGhost {
        rel: u8,
    },
    RemoveGhostRelation,
    MutateGhost {
        kind: u8,
    },
    BadPrimitive {
        n: u8,
    },
}

pub fn arb_op() -> impl Strategy<Value = Op> {
    use Op::*;
    prop_oneof![
        8 => (any::<u8>(), any::<u8>()).prop_map(|(n, p)| CreateNote { n, parent: p }),
        4 => (any::<u8>(), any::<u8>()).prop_map(|(n, t)| CreateDoc { n, text: t }),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(n, t)| CreateCode { n, text: t }),
        6 => (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(n, k, s, p)| CreatePrimitive { n, kind: k, size: s, parent: p }),
        3 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(n, e, s)| CreateRequirement { n, expr: e, sat: s }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(n, a)| CreateDecision { n, affects: a }),
        4 => (any::<u8>(), any::<bool>(), any::<bool>())
            .prop_map(|(p, c, b)| Delete { pick: p, cascade: c, by_name: b }),
        3 => (any::<u8>(), any::<u8>(), any::<bool>())
            .prop_map(|(p, n, b)| Rename { pick: p, n, by_name: b }),
        4 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(p, pa, v)| SetProperty { pick: p, path: pa, val: v }),
        3 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(p, c, v)| SetComponent { pick: p, comp: c, val: v }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(p, c)| RemoveComponent { pick: p, comp: c }),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(p, t)| AddTag { pick: p, tag: t }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(p, t)| RemoveTag { pick: p, tag: t }),
        5 => (any::<u8>(), any::<u8>(), any::<u8>(), any::<bool>())
            .prop_map(|(r, f, t, b)| AddRelation { rel: r, from: f, to: t, by_name: b }),
        3 => any::<u8>().prop_map(|p| RemoveRelation { pick: p }),
        3 => (any::<u8>(), any::<u8>(), any::<bool>())
            .prop_map(|(p, t, a)| SetDocText { pick: p, text: t, append: a }),
        3 => (any::<u8>(), any::<i8>(), any::<i8>(), any::<i8>())
            .prop_map(|(p, x, y, z)| Transform { pick: p, x, y, z }),
        1 => any::<u8>().prop_map(|n| ProjectRename { n }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(k, v)| SetMeta { key: k, val: v }),
        3 => (any::<bool>(), any::<u8>()).prop_map(|(a, p)| Evaluate { all: a, pick: p }),
        // ~1/6 of the stream is expected to fail — purity is asserted
        // on every failure, not just the happy path.
        2 => Just(BadSchema),
        2 => any::<u8>().prop_map(|n| PermDenied { n }),
        3 => any::<u8>().prop_map(|n| CreateBadParent { n }),
        2 => any::<u8>().prop_map(|r| AddRelationGhost { rel: r }),
        1 => Just(RemoveGhostRelation),
        2 => any::<u8>().prop_map(|k| MutateGhost { kind: k }),
        1 => any::<u8>().prop_map(|n| BadPrimitive { n }),
    ]
}

pub fn arb_ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(arb_op(), 8..48)
}

// ------------------------------------------------------------------ model

/// Live ids in creation order — the deterministic handle the generated
/// ops index into.
#[derive(Default)]
pub struct Model {
    pub live: Vec<ObjectId>,
    pub rels: Vec<RelationId>,
}

impl Model {
    /// Drop ids the engine no longer has (deletes, cascade deletes,
    /// undo) — keeps the model convergent without tracking effects.
    pub fn refresh(&mut self, e: &Engine) {
        self.live.retain(|id| e.get_object(*id).is_some());
        self.rels
            .retain(|id| e.project().relations.contains_key(id));
    }

    fn picked(&self, ix: u8) -> Option<ObjectId> {
        if self.live.is_empty() {
            None
        } else {
            Some(self.live[ix as usize % self.live.len()])
        }
    }

    /// `("id"|"name", value)` for object commands. Name refs are only
    /// emitted when unambiguous: `find_by_name` over duplicates is
    /// intentionally unordered.
    fn obj_key(&self, e: &Engine, ix: u8, by_name: bool) -> (String, String) {
        match self.picked(ix) {
            Some(id) => {
                if by_name {
                    let name = e.get_object(id).unwrap().name.clone();
                    let unique = self
                        .live
                        .iter()
                        .filter(|i| e.get_object(**i).is_some_and(|o| o.name == name))
                        .count()
                        == 1;
                    if unique {
                        return ("name".into(), name);
                    }
                }
                ("id".into(), id.to_string())
            }
            None => ("name".into(), GHOST_NAME.into()),
        }
    }

    /// Plain-string reference for relation endpoints (`from`/`to` take
    /// an id-or-name string, not a keyed field).
    fn endpoint(&self, e: &Engine, ix: u8, by_name: bool) -> String {
        self.obj_key(e, ix, by_name).1
    }
}

pub fn read_only_actor() -> Actor {
    Actor {
        id: worldos_kernel::ActorId::new("ro"),
        kind: ActorKind::Script,
        name: "ro".into(),
        permissions: PermissionSet::read_only(),
    }
}

// ------------------------------------------------------------------ executor

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Command succeeded: an `ok:true` record (+ops) is now in the open
    /// transaction, or was auto-committed to the journal.
    Committed,
    /// Handler ran and failed: no graph residue, but inside an
    /// explicit transaction an `ok:false` record stays in the open
    /// txn and is journaled by the later commit.
    FailedRecorded,
    /// Rejected before the transaction boundary — unknown command,
    /// schema validation, or actor permission. An open transaction
    /// observes nothing at all.
    Rejected,
}

/// Build the `(command_type, input)` pair for an op, or `None` when the
/// op is an actor-override call handled by the caller.
fn build(e: &Engine, m: &Model, op: &Op) -> (&'static str, Value) {
    use Op::*;
    match *op {
        CreateNote { n, parent } => {
            let mut input = json!({
                "type": "core:note",
                "name": obj_name(n),
                "tags": [format!("t{}", n % 4)],
                "components": {"app:props": {"prio": n % 5}},
            });
            // parent index → live parent 1/5 of the time, else none.
            if parent % 5 == 0
                && let Some(pid) = m.picked(parent)
            {
                input["parent"] = json!(pid.to_string());
            }
            ("object.create", input)
        }
        CreateDoc { n, text } => (
            "document.create",
            json!({"name": obj_name(n), "text": format!("body-{text}"), "format": "markdown"}),
        ),
        CreateCode { n, text } => (
            "code.create_file",
            json!({"name": obj_name(n), "language": "rust", "source": format!("// {text}")}),
        ),
        CreatePrimitive {
            n,
            kind,
            size,
            parent,
        } => {
            let mut input = json!({
                "kind": KIND_POOL[kind as usize % KIND_POOL.len()],
                "name": obj_name(n),
                "size": (size % 8) as f64 / 2.0 + 0.5,
                "position": [n % 4, 0, 0],
            });
            if parent % 5 == 1
                && let Some(pid) = m.picked(parent)
            {
                input["parent"] = json!(pid.to_string());
            }
            ("geometry.create_primitive", input)
        }
        CreateRequirement { n, expr, sat } => {
            let dep = obj_name(sat);
            let mut input = json!({"name": obj_name(n), "expression": req_expr(expr, &dep)});
            if sat % 4 == 0 && m.picked(sat).is_some() {
                input["satisfied_by"] = json!(m.endpoint(e, sat, false));
            }
            ("requirement.create", input)
        }
        CreateDecision { n, affects } => {
            let mut input = json!({
                "name": obj_name(n),
                "choice": format!("choice-{}", n % 3),
                "rationale": "property test",
            });
            if affects % 3 == 0 && m.picked(affects).is_some() {
                input["affects"] = json!([m.endpoint(e, affects, false)]);
            }
            ("decision.record", input)
        }
        Delete {
            pick,
            cascade,
            by_name,
        } => {
            let (k, v) = m.obj_key(e, pick, by_name);
            ("object.delete", json!({k: v, "cascade": cascade}))
        }
        Rename { pick, n, by_name } => {
            let (k, v) = m.obj_key(e, pick, by_name);
            let key = if k == "name" { "object" } else { "id" };
            ("object.rename", json!({key: v, "name": obj_name(n)}))
        }
        SetProperty { pick, path, val } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "object.set_property",
                json!({k: v, "component": "app:props",
                       "path": PATH_POOL[path as usize % PATH_POOL.len()],
                       "value": leaf(val)}),
            )
        }
        SetComponent { pick, comp, val } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "object.set_component",
                json!({k: v, "component": COMP_POOL[comp as usize % COMP_POOL.len()],
                       "version": (val % 3) + 1,
                       "data": {"a": leaf(val), "b": leaf(val.wrapping_add(1))}}),
            )
        }
        RemoveComponent { pick, comp } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "object.remove_component",
                json!({k: v, "component": RM_COMP_POOL[comp as usize % RM_COMP_POOL.len()]}),
            )
        }
        AddTag { pick, tag } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "object.add_tag",
                json!({k: v, "tag": format!("t{}", tag % 6)}),
            )
        }
        RemoveTag { pick, tag } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "object.remove_tag",
                json!({k: v, "tag": format!("t{}", tag % 6)}),
            )
        }
        AddRelation {
            rel,
            from,
            to,
            by_name,
        } => (
            "relation.add",
            json!({
                "type": REL_POOL[rel as usize % REL_POOL.len()],
                "from": m.endpoint(e, from, by_name),
                "to": m.endpoint(e, to, false),
                "properties": {"w": rel % 3},
            }),
        ),
        RemoveRelation { pick } => {
            let target = if m.rels.is_empty() {
                GHOST_ID.to_string()
            } else {
                m.rels[pick as usize % m.rels.len()].to_string()
            };
            ("relation.remove", json!({"id": target}))
        }
        SetDocText { pick, text, append } => {
            let (k, v) = m.obj_key(e, pick, false);
            let cmd = if append {
                "document.append_text"
            } else {
                "document.set_text"
            };
            (cmd, json!({k: v, "text": format!("t{text} ")}))
        }
        Transform { pick, x, y, z } => {
            let (k, v) = m.obj_key(e, pick, false);
            (
                "geometry.transform",
                json!({k: v,
                       "translate": [x as f64 / 4.0, y as f64 / 4.0, z as f64 / 4.0],
                       "scale": [(x % 3 + 3) as f64 / 3.0, 1.0, 1.0]}),
            )
        }
        ProjectRename { n } => ("project.rename", json!({"name": proj_name(n)})),
        SetMeta { key, val } => (
            "project.set_meta",
            json!({"key": KEY_POOL[key as usize % KEY_POOL.len()], "value": leaf(val)}),
        ),
        Evaluate { all, pick } => {
            if all {
                ("requirement.evaluate", json!({"all": true}))
            } else {
                let (k, v) = m.obj_key(e, pick, false);
                ("requirement.evaluate", json!({k: v}))
            }
        }
        // ------- deliberate failures -----------------------------------
        BadSchema => ("object.create", json!({"name": "missing-type"})),
        PermDenied { .. } => (
            "object.create",
            json!({"type": "core:note", "name": "denied"}),
        ),
        CreateBadParent { n } => (
            "object.create",
            json!({"type": "core:note", "name": obj_name(n), "parent": GHOST_NAME}),
        ),
        AddRelationGhost { rel } => (
            "relation.add",
            json!({"type": REL_POOL[rel as usize % REL_POOL.len()],
                   "from": GHOST_NAME, "to": GHOST_NAME}),
        ),
        RemoveGhostRelation => ("relation.remove", json!({"id": GHOST_ID})),
        MutateGhost { kind } => match kind % 5 {
            0 => ("object.rename", json!({"object": GHOST_NAME, "name": "x"})),
            1 => ("object.delete", json!({"id": GHOST_ID})),
            2 => (
                "object.set_property",
                json!({"name": GHOST_NAME, "component": "app:props", "path": "k", "value": 1}),
            ),
            3 => (
                "document.set_text",
                json!({"name": GHOST_NAME, "text": "x"}),
            ),
            _ => ("object.delete", json!({"id": "not-a-ulid"})),
        },
        BadPrimitive { n } => (
            "geometry.create_primitive",
            json!({"kind": "klein-bottle", "name": obj_name(n)}),
        ),
    }
}

/// Execute one op and assert the per-command invariants:
///
/// - committed command ⇒ exactly one new record appended at the undo
///   cursor (`History::push` truncates any redo tail, so the new length
///   is `cursor + 1`, not `len + 1`);
/// - failed command ⇒ graph AND journal are byte-identical to the
///   pre-command state — zero residue;
/// - the graph never contains dangling relations.
pub fn run_op(e: &mut Engine, m: &mut Model, op: &Op, in_txn: bool) -> Outcome {
    let before = canonical_project(e.project());
    let recs = e.history().records.len();
    let cursor = e.history().cursor;

    let (cmd, input) = build(e, m, op);
    let res: Result<CommandReceipt, EngineError> = match op {
        Op::PermDenied { .. } => e.execute_as(&read_only_actor(), cmd, input),
        _ => e.execute(cmd, input),
    };

    let outcome = match res {
        Ok(receipt) => {
            if in_txn {
                assert_eq!(
                    e.history().records.len(),
                    recs,
                    "command inside an open transaction must not commit ({op:?})"
                );
                assert_eq!(e.history().cursor, cursor);
            } else {
                assert_eq!(
                    e.history().records.len(),
                    cursor + 1,
                    "commit appends at the undo cursor, truncating the redo tail ({op:?})"
                );
                assert_eq!(e.history().cursor, cursor + 1);
            }
            if let Some(id) = receipt.output.get("id").and_then(|v| v.as_str()) {
                match op {
                    Op::AddRelation { .. } => {
                        if let Ok(rid) = id.parse::<RelationId>() {
                            m.rels.push(rid);
                        }
                    }
                    Op::CreateNote { .. }
                    | Op::CreateDoc { .. }
                    | Op::CreateCode { .. }
                    | Op::CreatePrimitive { .. }
                    | Op::CreateRequirement { .. }
                    | Op::CreateDecision { .. } => {
                        if let Ok(oid) = id.parse::<ObjectId>() {
                            m.live.push(oid);
                        }
                    }
                    _ => {}
                }
            }
            Outcome::Committed
        }
        Err(err) => {
            assert_eq!(
                canonical_project(e.project()),
                before,
                "failed op left graph residue: {op:?} ({err})"
            );
            assert_eq!(
                e.history().records.len(),
                recs,
                "failed op left journal residue: {op:?} ({err})"
            );
            assert_eq!(e.history().cursor, cursor);
            // Classify WHERE it failed: pre-transaction rejections
            // (unknown command, invalid input, actor permission —
            // checked before `open_txn` is touched) leave an explicit
            // transaction empty; anything later is journaled as
            // `ok:false` at commit. Mid-handler `PermissionDenied`
            // (capability checks) would blur this, but no generated
            // op reaches one — the capability-gated commands
            // (geometry.import/export, cad.*) aren't in `Op`.
            match &err {
                EngineError::Command(
                    CommandError::Unknown(_)
                    | CommandError::Validation { .. }
                    | CommandError::PermissionDenied { .. },
                ) => Outcome::Rejected,
                _ => Outcome::FailedRecorded,
            }
        }
    };
    m.refresh(e);
    assert!(
        e.project().dangling_relations().is_empty(),
        "dangling relation after {op:?}"
    );
    outcome
}

// ------------------------------------------------------------------ canonical forms

/// Order-independent serialization of semantic project state.
/// `meta` (timestamps, revisions) is included on purpose: undo restores
/// `before` verbatim and SQLite persists it verbatim, so any drift is a
/// real divergence, not tolerance noise.
pub fn canonical_project(p: &Project) -> Value {
    let mut objs: Vec<_> = p.objects.values().collect();
    objs.sort_by_key(|o| o.id);
    let mut rels: Vec<_> = p.relations.values().collect();
    rels.sort_by_key(|r| r.id);
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "schema_version": p.schema_version,
        "created_at": p.created_at,
        "settings": p.settings,
        "objects": objs,
        "relations": rels,
    })
}

/// Full journal state: every transaction record plus the undo cursor.
pub fn canonical_history(h: &History) -> Value {
    json!({
        "records": serde_json::to_value(&h.records).unwrap(),
        "cursor": h.cursor,
    })
}

/// Undo everything; the linear cursor reaches 0.
pub fn undo_all(e: &mut Engine) {
    while e.can_undo() {
        e.undo().expect("undo must not fail outside a transaction");
    }
}

/// Redo everything; the cursor reaches `records.len()`.
pub fn redo_all(e: &mut Engine) {
    while e.can_redo() {
        e.redo().expect("redo must not fail outside a transaction");
    }
}

/// `History::replay` must rebuild exactly the objects/relations/settings
/// the live project holds — the ops are the whole truth.
pub fn assert_replay_matches(e: &Engine) {
    let mut probe = Project::new("replay-probe");
    e.history().replay(&mut probe);
    let mut want_objs: Vec<_> = e.project().objects.values().collect();
    want_objs.sort_by_key(|o| o.id);
    let mut got_objs: Vec<_> = probe.objects.values().collect();
    got_objs.sort_by_key(|o| o.id);
    assert_eq!(
        serde_json::to_value(&got_objs).unwrap(),
        serde_json::to_value(&want_objs).unwrap(),
        "replayed objects diverge from live state"
    );
    let mut want_rels: Vec<_> = e.project().relations.values().collect();
    want_rels.sort_by_key(|r| r.id);
    let mut got_rels: Vec<_> = probe.relations.values().collect();
    got_rels.sort_by_key(|r| r.id);
    assert_eq!(
        serde_json::to_value(&got_rels).unwrap(),
        serde_json::to_value(&want_rels).unwrap(),
        "replayed relations diverge from live state"
    );
    assert_eq!(
        serde_json::to_value(&probe.settings).unwrap(),
        serde_json::to_value(&e.project().settings).unwrap(),
        "replayed settings diverge from live state"
    );
}
