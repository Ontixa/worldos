//! Mesh export (Forge, geometry depth): `geometry.export` writes a
//! deterministic binary STL/OBJ for `geom:*` primitives through the
//! command/transaction layer — permission-gated, traversal-guarded,
//! attributed in history. The file itself is an external effect outside
//! undo, like `worldos artifact export`.

use serde_json::json;
use std::path::Path;
use worldos_engine::Engine;
use worldos_kernel::actor::{Actor, Permission, PermissionSet};

fn stl_path(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().to_string()
}

fn actor_with(grants: &[&str]) -> Actor {
    let mut a = Actor::human("restricted");
    a.permissions = PermissionSet {
        grants: grants.iter().map(|g| Permission::new(*g)).collect(),
    };
    a
}

#[test]
fn export_writes_deterministic_binary_stl() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box", "size": 2.0}),
    )
    .unwrap();

    let p1 = stl_path(dir.path(), "a.stl");
    let out = e
        .execute("geometry.export", json!({"object": "box", "path": p1}))
        .unwrap();
    assert_eq!(out.output["format"], "stl");
    assert_eq!(out.output["triangles"], 12);
    assert_eq!(out.output["bytes"], 84 + 12 * 50);

    let bytes = std::fs::read(&p1).unwrap();
    assert_eq!(bytes.len(), 84 + 12 * 50);
    assert_eq!(
        u32::from_le_bytes(bytes[80..84].try_into().unwrap()),
        12,
        "facet count field"
    );
    // receipt digest is the file's real content hash
    assert_eq!(
        out.output["sha256"].as_str().unwrap(),
        worldos_artifact::ArtifactRef::of(&bytes).to_string()
    );

    // same input → byte-identical second export (new path, since
    // existing files are never overwritten)
    let p2 = stl_path(dir.path(), "b.stl");
    e.execute("geometry.export", json!({"object": "box", "path": p2}))
        .unwrap();
    assert_eq!(bytes, std::fs::read(&p2).unwrap());
}

#[test]
fn export_supports_all_primitive_kinds_and_obj() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    for (i, kind) in ["cube", "sphere", "cylinder", "cone", "torus", "plane"]
        .iter()
        .enumerate()
    {
        e.execute(
            "geometry.create_primitive",
            json!({"kind": kind, "name": format!("p{i}")}),
        )
        .unwrap();
        let path = stl_path(dir.path(), &format!("p{i}.stl"));
        let out = e
            .execute(
                "geometry.export",
                json!({"name": format!("p{i}"), "path": path}),
            )
            .unwrap();
        assert!(out.output["triangles"].as_u64().unwrap() >= 2, "{kind}");
        assert!(std::fs::metadata(&path).unwrap().len() > 84, "{kind}");
    }
    // OBJ is text with v/f lines
    let obj_path = stl_path(dir.path(), "p0.obj");
    e.execute(
        "geometry.export",
        json!({"object": "p0", "path": obj_path, "format": "obj"}),
    )
    .unwrap();
    let text = String::from_utf8(std::fs::read(&obj_path).unwrap()).unwrap();
    assert!(text.contains("\nv ") && text.contains("\nf "));
}

#[test]
fn export_denies_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box"}),
    )
    .unwrap();
    let path = stl_path(dir.path(), "denied.stl");

    // schema-level: no artifact.export grant → refused by the engine
    let reader = actor_with(&["project.read", "project.write"]);
    let err = e
        .execute_as(
            &reader,
            "geometry.export",
            json!({"object": "box", "path": &path}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("permission denied"), "{err}");
    assert!(!Path::new(&path).exists());

    // in-handler: artifact.export granted but no filesystem.write → the
    // command itself refuses before touching disk (agent/plugin default
    // grants look exactly like this)
    let agent_like = actor_with(&["project.*", "command.execute", "artifact.export"]);
    let err = e
        .execute_as(
            &agent_like,
            "geometry.export",
            json!({"object": "box", "path": &path}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("filesystem.write"), "{err}");
    assert!(!Path::new(&path).exists());

    // capability surface applies both checks to the calling actor
    let err = e
        .run_capability_as(
            &reader,
            "geometry.export",
            json!({"object": "box", "path": &path}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("permission denied"), "{err}");
    // and reaches the command for a fully-granted actor
    e.run_capability_as(
        &Actor::human("op"),
        "geometry.export",
        json!({"object": "box", "path": &path}),
    )
    .unwrap();
    assert!(Path::new(&path).is_file());
}

#[test]
fn export_rejects_traversal_and_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box"}),
    )
    .unwrap();

    for bad in ["../escape.stl", "a/../b.stl", "..\\escape.stl"] {
        let err = e
            .execute("geometry.export", json!({"object": "box", "path": bad}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("traversal"), "{bad}: {err}");
    }
    assert!(!dir.path().parent().unwrap().join("escape.stl").exists());

    // existing files are never overwritten
    let existing = stl_path(dir.path(), "taken.stl");
    std::fs::write(&existing, b"keep me").unwrap();
    let err = e
        .execute(
            "geometry.export",
            json!({"object": "box", "path": &existing}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("never overwritten"), "{err}");
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
}

#[test]
fn export_fails_closed_on_wrong_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "memo", "components": {}}),
    )
    .unwrap();
    let path = stl_path(dir.path(), "memo.stl");
    assert!(
        e.execute("geometry.export", json!({"object": "memo", "path": &path}))
            .is_err()
    );
    assert!(!Path::new(&path).exists());

    // unknown objects fail at resolution, before any fs access
    let ghost = stl_path(dir.path(), "ghost.stl");
    assert!(
        e.execute(
            "geometry.export",
            json!({"object": "ghost", "path": &ghost})
        )
        .is_err()
    );
    assert!(!Path::new(&ghost).exists());
}

#[test]
fn export_is_attributed_history_but_external_effect() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box"}),
    )
    .unwrap();
    let path = stl_path(dir.path(), "audit.stl");
    e.execute("geometry.export", json!({"object": "box", "path": &path}))
        .unwrap();

    // the journal records the export under the acting actor, with the
    // output digest — the audit trail lives in history, not the graph
    let rec = e.history().records.last().unwrap();
    assert_eq!(rec.label, "geometry.export");
    assert_eq!(rec.actor.0, "local-user");
    assert!(rec.ops.is_empty(), "file write emits no StateOps");
    assert!(rec.commands[0].output["sha256"].is_string());

    // undo does not remove the external file (same contract as
    // `worldos artifact export` — documented in docs/cad-cli.md)
    e.undo().unwrap();
    assert!(Path::new(&path).is_file());
}
