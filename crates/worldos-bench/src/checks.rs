//! `expect` check kinds — every check is a deterministic predicate over
//! the step receipt, its error text, or live engine state. No clocks,
//! no randomness; the only I/O is the artifact/file digests the check
//! itself exists to verify.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worldos_engine::Engine;
use worldos_kernel::Severity;
use worldos_kernel::ids::ObjectId;
use worldos_kernel::project::Project;

use crate::{dig, interp_s, interpolate};

/// One expect-check inside a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// Step must succeed.
    Ok,
    /// Step must fail; `contains` optionally matches the error text.
    Error {
        #[serde(default)]
        contains: Option<String>,
    },
    /// `path` (dot-separated into receipt output) must equal `value`.
    /// `value` is `${var.path}`-interpolated like step inputs.
    Eq { path: String, value: Value },
    /// Numeric `path` must be within `rel` relative error of `value`.
    Approx {
        path: String,
        value: f64,
        #[serde(default = "default_rel")]
        rel: f64,
    },
    /// `path` must exist and be non-null in the output.
    Present { path: String },
    /// No object named `name` may exist in the project afterwards.
    QueryAbsent { name: String },
    /// An object named `name` must exist afterwards; `path` optionally
    /// dot-resolves into its component data and must equal `value`.
    QueryObject {
        name: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        value: Option<Value>,
    },
    /// Object `name` must exist and its component `path`
    /// (`component.field.…`) must be absent or null — e.g. the dropped
    /// `cad:shape.step` ref after undoing `cad.export_step`.
    FieldAbsent { name: String, path: String },
    /// History invariants over the committed transaction log: total
    /// `records` (including the undone tail), applied `cursor`, and
    /// undo/redo availability. Assert any subset.
    History {
        #[serde(default)]
        records: Option<usize>,
        #[serde(default)]
        cursor: Option<usize>,
        #[serde(default)]
        can_undo: Option<bool>,
        #[serde(default)]
        can_redo: Option<bool>,
    },
    /// Canonical SHA-256 digest of the whole project graph. `save`
    /// stores it under a name; `eq` asserts equality with a stored
    /// digest — the exactness check for undo/redo, failed-transaction
    /// purity, and save/reopen round-trips.
    Snapshot {
        #[serde(default)]
        save: Option<String>,
        #[serde(default)]
        eq: Option<String>,
    },
    /// Count relations matching all given filters. `from`/`to` accept
    /// an object name or id (`${…}`-interpolated); an unresolvable
    /// endpoint matches nothing, so only `count: 0` passes then.
    RelationCount {
        count: usize,
        #[serde(default, rename = "type")]
        type_id: Option<String>,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        to: Option<String>,
    },
    /// Every stored relation endpoint must resolve — the graph's
    /// referential-integrity invariant.
    NoDanglingRelations,
    /// Count objects, optionally restricted to one `type`.
    ObjectCount {
        count: usize,
        #[serde(default, rename = "type")]
        type_id: Option<String>,
    },
    /// An artifact ref (`sha256:<hex>`) — literal `ref` or dug from the
    /// receipt `path` — must exist in the project file's artifact
    /// sidecar and re-hash to itself (the store's verify-on-read).
    ArtifactVerified {
        #[serde(default)]
        path: Option<String>,
        #[serde(default, rename = "ref")]
        artifact_ref: Option<String>,
    },
    /// A file written by an export step must exist and hash to the
    /// claimed digest — literal `sha256` or `sha256_path` into the
    /// receipt. `path`/`sha256` are `${…}`-interpolated.
    FileDigest {
        path: String,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        sha256_path: Option<String>,
    },
    /// Engine validators must report zero error diagnostics;
    /// `max_warnings` optionally bounds warning diagnostics.
    Valid {
        #[serde(default)]
        max_warnings: Option<usize>,
    },
}

fn default_rel() -> f64 {
    1e-4
}

/// Evaluate one check. `ok`/`out`/`err` describe the step's receipt;
/// `vars` carries `save:`-bound outputs for `${…}` interpolation;
/// `snapshots` stores named state digests across steps.
pub(crate) fn eval_check(
    check: &Check,
    engine: &Engine,
    ok: bool,
    out: &Value,
    err: &str,
    vars: &HashMap<String, Value>,
    snapshots: &mut HashMap<String, String>,
) -> (bool, String) {
    match check {
        Check::Ok => (
            ok,
            if ok {
                "ok".into()
            } else {
                format!("failed: {err}")
            },
        ),
        Check::Error { contains } => {
            if ok {
                (false, "expected error, step succeeded".into())
            } else if let Some(c) = contains {
                (
                    err.contains(c.as_str()),
                    format!("error `{err}` contains `{c}`"),
                )
            } else {
                (true, format!("failed as expected: {err}"))
            }
        }
        Check::Eq { path, value } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            let got = dig(out, path);
            let want = interpolate(value, vars);
            (
                got == Some(&want),
                format!("{path} = {got:?} (want {want:?})"),
            )
        }
        Check::Approx { path, value, rel } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            match dig(out, path).and_then(|v| v.as_f64()) {
                Some(got) => {
                    let ok = worldos_cad::approx_relative(got, *value)
                        || (got - value).abs() <= rel * value.abs().max(1.0);
                    (ok, format!("{path} = {got} (want ~{value} rel {rel})"))
                }
                None => (false, format!("{path} not a number")),
            }
        }
        Check::Present { path } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            (
                dig(out, path).is_some(),
                format!("{path} present = {}", dig(out, path).is_some()),
            )
        }
        Check::QueryAbsent { name } => {
            let name = interp_s(name, vars);
            let absent = engine.project().find_by_name(&name).is_none();
            (absent, format!("object `{name}` absent = {absent}"))
        }
        Check::QueryObject { name, path, value } => {
            let name = interp_s(name, vars);
            let Some(obj) = engine.project().find_by_name(&name) else {
                return (false, format!("object `{name}` not found"));
            };
            if let (Some(p), Some(want)) = (path, value) {
                // path is "component.field.subfield": resolve into the
                // object's component map
                let mut parts = p.splitn(2, '.');
                let comp = parts.next().unwrap_or("");
                let rest = parts.next().unwrap_or("");
                let found = obj.components.get(comp).and_then(|c| {
                    if rest.is_empty() {
                        Some(&c.data)
                    } else {
                        dig(&c.data, rest)
                    }
                });
                let want = interpolate(want, vars);
                let matched = found == Some(&want);
                (matched, format!("`{name}` {p} = {found:?} (want {want:?})"))
            } else {
                (true, format!("object `{name}` exists"))
            }
        }
        Check::FieldAbsent { name, path } => {
            let name = interp_s(name, vars);
            let Some(obj) = engine.project().find_by_name(&name) else {
                return (false, format!("object `{name}` not found"));
            };
            let mut parts = path.splitn(2, '.');
            let comp = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("");
            let found = obj.components.get(comp).and_then(|c| {
                if rest.is_empty() {
                    Some(&c.data)
                } else {
                    dig(&c.data, rest)
                }
            });
            let absent = found.is_none_or(|v| v.is_null());
            (absent, format!("`{name}` {path} = {found:?} (want absent)"))
        }
        Check::History {
            records,
            cursor,
            can_undo,
            can_redo,
        } => {
            let h = engine.history();
            let mut bad = Vec::new();
            if let Some(w) = records
                && h.records.len() != *w
            {
                bad.push(format!("records={} want {w}", h.records.len()));
            }
            if let Some(w) = cursor
                && h.cursor != *w
            {
                bad.push(format!("cursor={} want {w}", h.cursor));
            }
            if let Some(w) = can_undo
                && engine.can_undo() != *w
            {
                bad.push(format!("can_undo={} want {w}", engine.can_undo()));
            }
            if let Some(w) = can_redo
                && engine.can_redo() != *w
            {
                bad.push(format!("can_redo={} want {w}", engine.can_redo()));
            }
            if bad.is_empty() {
                (
                    true,
                    format!(
                        "records={} cursor={} undo={} redo={}",
                        h.records.len(),
                        h.cursor,
                        engine.can_undo(),
                        engine.can_redo()
                    ),
                )
            } else {
                (false, bad.join("; "))
            }
        }
        Check::Snapshot { save, eq } => {
            let digest = project_digest(engine.project());
            if let Some(name) = save {
                snapshots.insert(name.clone(), digest.clone());
            }
            match eq {
                Some(name) => match snapshots.get(name) {
                    Some(want) => (
                        digest == *want,
                        if digest == *want {
                            format!("state digest matches `{name}` ({digest})")
                        } else {
                            format!("state digest {digest} != `{name}` {want}")
                        },
                    ),
                    None => (false, format!("no snapshot named `{name}`")),
                },
                None => (true, format!("state digest {digest}")),
            }
        }
        Check::RelationCount {
            count,
            type_id,
            from,
            to,
        } => {
            let p = engine.project();
            let from = from.as_deref().map(|s| interp_s(s, vars));
            let to = to.as_deref().map(|s| interp_s(s, vars));
            // A given-but-unresolvable endpoint matches nothing.
            let from_id = from.as_deref().map(|s| resolve_endpoint(p, s));
            let to_id = to.as_deref().map(|s| resolve_endpoint(p, s));
            let n = p
                .relations
                .values()
                .filter(|r| {
                    type_id.as_deref().is_none_or(|t| r.type_id == t)
                        && from_id.is_none_or(|f| f.is_some_and(|f| r.from == f))
                        && to_id.is_none_or(|t| t.is_some_and(|t| r.to == t))
                })
                .count();
            (
                n == *count,
                format!(
                    "{n} relation(s) match type={type_id:?} from={from:?} to={to:?} (want {count})"
                ),
            )
        }
        Check::NoDanglingRelations => {
            let n = engine.project().dangling_relations().len();
            (n == 0, format!("{n} dangling relation(s)"))
        }
        Check::ObjectCount { count, type_id } => {
            let n = match type_id {
                Some(t) => engine.project().objects_of_type(t).count(),
                None => engine.project().objects.len(),
            };
            (
                n == *count,
                format!(
                    "{n} object(s){} (want {count})",
                    type_id
                        .as_ref()
                        .map(|t| format!(" of type `{t}`"))
                        .unwrap_or_default()
                ),
            )
        }
        Check::ArtifactVerified { path, artifact_ref } => {
            let rs = if let Some(p) = path {
                match dig(out, p).and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return (false, format!("{p} not a string in output")),
                }
            } else if let Some(r) = artifact_ref {
                interp_s(r, vars)
            } else {
                return (false, "artifact_verified needs `path` or `ref`".into());
            };
            let aref: worldos_artifact::ArtifactRef = match rs.parse() {
                Ok(r) => r,
                Err(e) => return (false, format!("bad artifact ref `{rs}`: {e}")),
            };
            let Some(project_path) = engine.path() else {
                return (false, "engine has no backing file".into());
            };
            // Check the on-disk sidecar of the *current* project file —
            // after `save_as` this is the migrated store, which is the
            // point of the check.
            let store = match worldos_artifact::ArtifactStore::for_project(project_path) {
                Ok(s) => s,
                Err(e) => return (false, format!("artifact store: {e}")),
            };
            match store.verify(&aref) {
                Ok(true) => (true, format!("artifact {rs} present + re-hash verified")),
                Ok(false) => (false, format!("artifact {rs} fails re-hash")),
                Err(e) => (false, format!("artifact {rs}: {e}")),
            }
        }
        Check::FileDigest {
            path,
            sha256,
            sha256_path,
        } => {
            let p = interp_s(path, vars);
            let bytes = match std::fs::read(&p) {
                Ok(b) => b,
                Err(e) => return (false, format!("cannot read `{p}`: {e}")),
            };
            let actual = worldos_artifact::ArtifactRef::of(&bytes).to_string();
            let want = if let Some(s) = sha256 {
                Some(interp_s(s, vars))
            } else {
                sha256_path
                    .as_deref()
                    .and_then(|sp| dig(out, sp))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            };
            match want {
                Some(w) => (actual == w, format!("`{p}` sha256 {actual} (want {w})")),
                None => (false, "file_digest needs `sha256` or `sha256_path`".into()),
            }
        }
        Check::Valid { max_warnings } => {
            let rep = engine.validate();
            let errors = rep
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .count();
            let warnings = rep
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Warning)
                .count();
            let pass = errors == 0 && max_warnings.is_none_or(|m| warnings <= m);
            (
                pass,
                format!(
                    "{errors} error(s), {warnings} warning(s){}",
                    max_warnings
                        .map(|m| format!(" (max {m})"))
                        .unwrap_or_default()
                ),
            )
        }
    }
}

/// Name-or-id resolution for relation endpoints in checks.
fn resolve_endpoint(p: &Project, s: &str) -> Option<ObjectId> {
    if let Ok(id) = s.parse::<ObjectId>() {
        return Some(id);
    }
    p.find_by_name(s).map(|o| o.id)
}

/// Canonical fingerprint of the whole project graph: id + name +
/// settings + every object and relation serialized in id order, hashed
/// with SHA-256. Deterministic across undo/redo and save/reopen because
/// the recorded before/after states are byte-identical clones and the
/// on-disk round-trip preserves them.
pub fn project_digest(p: &Project) -> String {
    let mut buf = Vec::new();
    buf.extend_from_slice(p.id.to_string().as_bytes());
    buf.push(0);
    buf.extend_from_slice(p.name.as_bytes());
    buf.push(0);
    buf.extend_from_slice(
        serde_json::to_string(&p.settings)
            .unwrap_or_default()
            .as_bytes(),
    );
    let mut objs: Vec<_> = p.objects.values().collect();
    objs.sort_by_key(|o| o.id);
    for o in objs {
        buf.push(0);
        buf.extend_from_slice(serde_json::to_string(o).unwrap_or_default().as_bytes());
    }
    let mut rels: Vec<_> = p.relations.values().collect();
    rels.sort_by_key(|r| r.id);
    for r in rels {
        buf.push(0);
        buf.extend_from_slice(serde_json::to_string(r).unwrap_or_default().as_bytes());
    }
    worldos_artifact::ArtifactRef::of(&buf).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use worldos_kernel::{ActorId, Object, Relation};

    fn test_engine() -> Engine {
        Engine::new("t")
    }

    #[test]
    fn snapshot_digest_is_stable_under_undo() {
        let mut e = test_engine();
        let mut snaps = HashMap::new();
        let vars = HashMap::new();
        e.execute("object.create", json!({"type": "core:note", "name": "a"}))
            .unwrap();
        let (p1, _) = eval_check(
            &Check::Snapshot {
                save: Some("s0".into()),
                eq: None,
            },
            &e,
            true,
            &Value::Null,
            "",
            &vars,
            &mut snaps,
        );
        assert!(p1);
        e.execute("object.create", json!({"type": "core:note", "name": "b"}))
            .unwrap();
        e.undo().unwrap();
        let (p2, detail) = eval_check(
            &Check::Snapshot {
                save: None,
                eq: Some("s0".into()),
            },
            &e,
            true,
            &Value::Null,
            "",
            &vars,
            &mut snaps,
        );
        assert!(p2, "{detail}");
    }

    #[test]
    fn history_and_counts_track_the_graph() {
        let mut e = test_engine();
        let vars = HashMap::new();
        let mut snaps = HashMap::new();
        e.execute("object.create", json!({"type": "core:note", "name": "a"}))
            .unwrap();
        let bid = e
            .execute("object.create", json!({"type": "core:note", "name": "b"}))
            .unwrap()
            .output["id"]
            .as_str()
            .unwrap()
            .to_string();
        e.execute(
            "relation.add",
            json!({"type": "core:depends-on", "from": "a", "to": "b"}),
        )
        .unwrap();

        let run = |e: &Engine, c: &Check, snaps: &mut HashMap<String, String>| {
            eval_check(c, e, true, &Value::Null, "", &vars, snaps)
        };
        assert!(
            run(
                &e,
                &Check::History {
                    records: Some(3),
                    cursor: Some(3),
                    can_undo: Some(true),
                    can_redo: Some(false),
                },
                &mut snaps
            )
            .0
        );
        assert!(
            run(
                &e,
                &Check::RelationCount {
                    count: 1,
                    type_id: Some("core:depends-on".into()),
                    from: Some("a".into()),
                    to: Some(bid.clone()),
                },
                &mut snaps
            )
            .0
        );
        assert!(run(&e, &Check::NoDanglingRelations, &mut snaps).0);
        assert!(
            run(
                &e,
                &Check::ObjectCount {
                    count: 2,
                    type_id: Some("core:note".into())
                },
                &mut snaps
            )
            .0
        );

        // Deleting an object removes its relations — integrity holds.
        e.execute("object.delete", json!({"name": "b"})).unwrap();
        assert!(
            run(
                &e,
                &Check::RelationCount {
                    count: 0,
                    type_id: None,
                    from: None,
                    to: None
                },
                &mut snaps
            )
            .0
        );
        assert!(run(&e, &Check::NoDanglingRelations, &mut snaps).0);
        assert!(run(&e, &Check::Valid { max_warnings: None }, &mut snaps).0);
    }

    #[test]
    fn digest_covers_relations_and_settings() {
        let actor = ActorId::new("t");
        let mut p = Project::new("d");
        let a = Object::new("core:note", "a", &actor);
        let b = Object::new("core:note", "b", &actor);
        p.objects.insert(a.id, a.clone());
        p.objects.insert(b.id, b.clone());
        let d0 = project_digest(&p);
        p.relations.insert(
            worldos_kernel::RelationId::new(),
            Relation::new("core:references", a.id, b.id, &actor),
        );
        assert_ne!(d0, project_digest(&p));
    }
}
