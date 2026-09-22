//! Mesh import (Forge, geometry depth): `geometry.import` reads a
//! binary STL / OBJ file and creates a `geom:mesh` object through the
//! command/transaction layer — permission-gated, traversal-guarded,
//! attributed in history, exactly undoable. The imported mesh is real
//! geometry: `geometry.measure` reports its volume/area/bbox,
//! requirement terms `volume()`/`area()` evaluate it, and
//! `geometry.export` re-exports it (byte-identical for our own files).

use serde_json::json;
use std::path::Path;
use worldos_engine::Engine;
use worldos_kernel::actor::{Actor, Permission, PermissionSet};

fn out_path(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().to_string()
}

fn actor_with(grants: &[&str]) -> Actor {
    let mut a = Actor::human("restricted");
    a.permissions = PermissionSet {
        grants: grants.iter().map(|g| Permission::new(*g)).collect(),
    };
    a
}

/// Export `object` to `path` and return the written bytes.
fn export(e: &mut Engine, object: &str, path: &str, format: &str) -> Vec<u8> {
    e.execute(
        "geometry.export",
        json!({"object": object, "path": path, "format": format}),
    )
    .unwrap();
    std::fs::read(path).unwrap()
}

#[test]
fn import_roundtrips_binary_stl_byte_identically() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box", "size": 2.0}),
    )
    .unwrap();
    let stl = out_path(dir.path(), "box.stl");
    let original = export(&mut e, "box", &stl, "stl");

    let out = e.execute("geometry.import", json!({"path": &stl})).unwrap();
    assert_eq!(out.output["type"], "geom:mesh");
    assert_eq!(out.output["name"], "box"); // default name = file stem
    assert_eq!(out.output["format"], "stl");
    assert_eq!(out.output["triangles"], 12);
    assert_eq!(out.output["vertices"], 8, "soup welded to indexed mesh");
    assert_eq!(
        out.output["sha256"].as_str().unwrap(),
        worldos_artifact::ArtifactRef::of(&original).to_string()
    );

    // the imported object measures its stored mesh — a tessellated
    // cube's facets are exact, so volume/area match analytic values.
    // ("box" now names two objects; address the import by id.)
    let id = out.output["id"].as_str().unwrap();
    let m = e
        .run_capability("geometry.measure", json!({"id": id}))
        .unwrap();
    assert_eq!(m["kind"], "mesh");
    assert!((m["volume"].as_f64().unwrap() - 8.0).abs() < 1e-9);
    assert!((m["surface_area"].as_f64().unwrap() - 24.0).abs() < 1e-9);
    assert_eq!(m["bbox"]["min"], json!([-1.0, -1.0, -1.0]));

    // re-exporting the imported mesh reproduces the file byte-for-byte:
    // welding preserves each facet's coordinates and winding
    let re = out_path(dir.path(), "re.stl");
    e.execute("geometry.export", json!({"object": id, "path": &re}))
        .unwrap();
    assert_eq!(std::fs::read(&re).unwrap(), original);
}

#[test]
fn import_roundtrips_obj_and_matches_tessellation_tolerance() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cylinder", "name": "cyl", "size": [2.0, 2.0, 5.0]}),
    )
    .unwrap();
    let obj_path = out_path(dir.path(), "cyl.obj");
    let original = export(&mut e, "cyl", &obj_path, "obj");

    let out = e
        .execute(
            "geometry.import",
            json!({"path": &obj_path, "name": "cyl-imported"}),
        )
        .unwrap();
    let id = out.output["id"].as_str().unwrap();
    assert_eq!(out.output["triangles"], 4 * 32); // sides + caps

    let m = e
        .run_capability("geometry.measure", json!({"id": id}))
        .unwrap();
    // analytic: r=1 h=5 → V = 5π ≈ 15.708, A = 12π ≈ 37.699
    let analytic = e
        .run_capability("geometry.measure", json!({"name": "cyl"}))
        .unwrap();
    for f in ["volume", "surface_area"] {
        let a = analytic[f].as_f64().unwrap();
        let b = m[f].as_f64().unwrap();
        assert!(
            (b / a - 1.0).abs() < 0.02,
            "{f}: mesh {b} vs analytic {a} — outside 2% tessellation tolerance"
        );
    }

    // OBJ re-export is byte-identical (positions round-trip exactly
    // through f64 shortest-representation text)
    let re = out_path(dir.path(), "re.obj");
    e.execute(
        "geometry.export",
        json!({"object": id, "path": &re, "format": "obj"}),
    )
    .unwrap();
    assert_eq!(std::fs::read(&re).unwrap(), original);

    // STL too: normals are computed from the quantized f32 vertices
    // actually written, so even a curved mesh re-exports identically
    let stl = out_path(dir.path(), "cyl2.stl");
    let stl_bytes = export(&mut e, "cyl", &stl, "stl");
    let id2 = e
        .execute(
            "geometry.import",
            json!({"path": &stl, "name": "cyl-rt-stl"}),
        )
        .unwrap();
    let re2 = out_path(dir.path(), "re2.stl");
    e.execute(
        "geometry.export",
        json!({"object": id2.output["id"].as_str().unwrap(), "path": &re2}),
    )
    .unwrap();
    assert_eq!(std::fs::read(&re2).unwrap(), stl_bytes);
}

#[test]
fn import_honors_transform_and_requirement_terms() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box", "size": 2.0}),
    )
    .unwrap();
    let stl = out_path(dir.path(), "box.stl");
    export(&mut e, "box", &stl, "stl");

    let out = e
        .execute(
            "geometry.import",
            json!({"path": &stl, "name": "shifted", "position": [10.0, 0.0, 0.0]}),
        )
        .unwrap();
    let id = out.output["id"].as_str().unwrap().to_string();
    let m = e
        .run_capability("geometry.measure", json!({"id": id}))
        .unwrap();
    assert_eq!(m["bbox"]["min"], json!([9.0, -1.0, -1.0]));
    assert!((m["volume"].as_f64().unwrap() - 8.0).abs() < 1e-9);

    // requirement measure terms read the same mesh geometry
    e.execute(
        "requirement.create",
        json!({"name": "vol", "expression": "volume(\"shifted\") >= 8"}),
    )
    .unwrap();
    e.execute("requirement.evaluate", json!({"name": "vol"}))
        .unwrap();
    let req = e.find_object("vol").unwrap();
    assert_eq!(
        req.component_data("core:requirement-status").unwrap()["status"],
        "pass"
    );

    // scale applies to the stored mesh (uniform ×2 → volume ×8)
    e.execute(
        "geometry.transform",
        json!({"id": id, "scale": [2.0, 2.0, 2.0]}),
    )
    .unwrap();
    let m = e
        .run_capability("geometry.measure", json!({"id": id}))
        .unwrap();
    assert!((m["volume"].as_f64().unwrap() - 64.0).abs() < 1e-9);
    assert!((m["surface_area"].as_f64().unwrap() - 96.0).abs() < 1e-9);
}

#[test]
fn import_denies_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box"}),
    )
    .unwrap();
    let stl = out_path(dir.path(), "box.stl");
    export(&mut e, "box", &stl, "stl");
    let objects_before = e.project().objects.len();
    let history_before = e.history().records.len();

    // schema-level: no artifact.import grant → refused by the engine
    let reader = actor_with(&["project.read", "project.write"]);
    let err = e
        .execute_as(&reader, "geometry.import", json!({"path": &stl}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("permission denied"), "{err}");

    // in-handler: artifact.import granted but no filesystem.read → the
    // command refuses before touching disk (agent/plugin default grants
    // look exactly like this)
    let agent_like = actor_with(&["project.*", "command.execute", "artifact.import"]);
    let err = e
        .execute_as(&agent_like, "geometry.import", json!({"path": &stl}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("filesystem.read"), "{err}");

    // capability surface applies both checks to the calling actor
    let err = e
        .run_capability_as(&reader, "geometry.import", json!({"path": &stl}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("permission denied"), "{err}");
    e.run_capability_as(
        &Actor::human("op"),
        "geometry.import",
        json!({"path": &stl}),
    )
    .unwrap();

    // failures rolled back: nothing was created, nothing was journaled
    assert_eq!(e.project().objects.len(), objects_before + 1);
    assert_eq!(e.history().records.len(), history_before + 1);
}

#[test]
fn import_fails_closed_on_bad_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    let objects_before = e.project().objects.len();
    let history_before = e.history().records.len();

    // missing file
    let missing = out_path(dir.path(), "ghost.stl");
    let err = e
        .execute("geometry.import", json!({"path": &missing}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot read"), "{err}");

    // unsupported extension, and explicit bogus format
    let step = out_path(dir.path(), "part.step");
    std::fs::write(&step, b"ISO-10303-21;").unwrap();
    let err = e
        .execute("geometry.import", json!({"path": &step}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot infer"), "{err}");
    // an out-of-enum format is rejected by schema validation before the
    // handler ever runs
    let err = e
        .execute("geometry.import", json!({"path": &step, "format": "step"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("enum"), "{err}");

    // malformed: truncated binary STL (declares more facets than it holds)
    let trunc = out_path(dir.path(), "trunc.stl");
    let mut bytes = {
        e.execute(
            "geometry.create_primitive",
            json!({"kind": "cube", "name": "tmp"}),
        )
        .unwrap();
        let p = out_path(dir.path(), "tmp.stl");
        export(&mut e, "tmp", &p, "stl")
    };
    bytes.truncate(100); // 84-byte header + 16 bytes of one facet
    std::fs::write(&trunc, &bytes).unwrap();
    let err = e
        .execute("geometry.import", json!({"path": &trunc}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("truncated"), "{err}");

    // malformed: ASCII STL detected and refused
    let ascii = out_path(dir.path(), "ascii.stl");
    std::fs::write(&ascii, b"solid x\n facet normal 0 0 1\n endsolid x\n").unwrap();
    let err = e
        .execute("geometry.import", json!({"path": &ascii}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("ASCII"), "{err}");

    // traversal is refused before any filesystem access
    for bad in ["../escape.stl", "a/../b.stl", "..\\escape.stl"] {
        let err = e
            .execute("geometry.import", json!({"path": bad}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("traversal"), "{bad}: {err}");
    }

    // every failure above rolled its auto-transaction back: the only
    // new object/history entry is the `tmp` primitive + its export
    let extra_objects: Vec<_> = e
        .project()
        .objects
        .values()
        .filter(|o| o.type_id.0 == "geom:mesh")
        .collect();
    assert!(extra_objects.is_empty(), "failed imports left no objects");
    assert_eq!(
        e.project().objects.len(),
        objects_before + 1,
        "only `tmp` was created"
    );
    assert_eq!(
        e.history().records.len(),
        history_before + 2,
        "only create + export committed"
    );
}

#[test]
fn import_is_undoable_attributed_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "box"}),
    )
    .unwrap();
    let stl = out_path(dir.path(), "box.stl");
    export(&mut e, "box", &stl, "stl");

    let out = e
        .execute("geometry.import", json!({"path": &stl, "name": "imported"}))
        .unwrap();
    let id = out.output["id"].as_str().unwrap().to_string();

    // attributed record with the source digest in the output
    let rec = e.history().records.last().unwrap();
    assert_eq!(rec.label, "geometry.import");
    assert_eq!(rec.actor.0, "local-user");
    assert!(!rec.ops.is_empty(), "import writes the object as StateOps");
    assert!(rec.commands[0].output["sha256"].is_string());
    // provenance rides in the component
    let obj = e.get_object(id.parse().unwrap()).unwrap();
    let src = &obj.component_data("geom:mesh").unwrap()["source"];
    assert_eq!(src["sha256"], out.output["sha256"]);
    assert_eq!(src["format"], "stl");

    // undo removes exactly the imported object
    e.undo().unwrap();
    assert!(e.find_object("imported").is_none());
    e.redo().unwrap();
    assert!(e.find_object("imported").is_some());

    // importing the same file twice yields identical component data
    let a = e
        .execute("geometry.import", json!({"path": &stl, "name": "a"}))
        .unwrap();
    let b = e
        .execute("geometry.import", json!({"path": &stl, "name": "b"}))
        .unwrap();
    let ca = e
        .get_object(a.output["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .component_data("geom:mesh")
        .unwrap();
    let cb = e
        .get_object(b.output["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .component_data("geom:mesh")
        .unwrap();
    assert_eq!(ca["positions"], cb["positions"]);
    assert_eq!(ca["indices"], cb["indices"]);
    // the source file is untouched by all of this
    assert_eq!(std::fs::read(&stl).unwrap().len(), 84 + 12 * 50);
}
