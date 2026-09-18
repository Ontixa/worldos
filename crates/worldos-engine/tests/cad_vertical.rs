//! Vertical slice proof (Forge slice 1):
//!
//!   cad.create_box → kernel-verified measure → STEP export → reimport →
//!   undo/redo → save → reopen → regenerate → same verified semantic
//!   result.
//!
//! Everything goes through `Engine::execute` — the same path CLI, RPC,
//! MCP, SDKs and agents use.

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use worldos_adapter_cadrum::CadrumKernel;
use worldos_cad::{CadKernel, approx_relative};
use worldos_engine::Engine;

fn sidecar(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".artifacts");
    s.into()
}

fn blob_bytes(dir: &Path, sha_ref: &str) -> Vec<u8> {
    let hex = sha_ref.strip_prefix("sha256:").unwrap_or(sha_ref);
    std::fs::read(dir.join("objects").join(&hex[..2]).join(hex)).expect("artifact blob missing")
}

#[test]
fn create_measure_step_reimport_undo_redo_save_reopen_regenerate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.worldos");

    let mut engine = Engine::create("cad-demo", &path).unwrap();
    engine
        .attach_cad(Arc::new(CadrumKernel::new()))
        .expect("attach cad");

    // --- create ------------------------------------------------------------
    let out = engine
        .execute(
            "cad.create_box",
            json!({"size_mm": [50.0, 40.0, 20.0], "name": "block"}),
        )
        .unwrap();
    let id = out.output["id"].as_str().unwrap().to_string();
    let brep_ref = out.output["brep"].as_str().unwrap().to_string();
    assert!(brep_ref.starts_with("sha256:"));
    assert!(approx_relative(
        out.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0
    ));
    assert_eq!(out.output["topology"]["faces"].as_u64().unwrap(), 6);
    assert_eq!(out.output["topology"]["edges"].as_u64().unwrap(), 12);
    assert!(out.output["topology"]["is_valid"].as_bool().unwrap());

    // artifact really on disk in the sidecar
    let art_dir = sidecar(&path);
    let brep_bytes = blob_bytes(&art_dir, &brep_ref);
    assert!(!brep_bytes.is_empty());

    // --- kernel-verified measure --------------------------------------------
    let m = engine
        .execute("cad.measure", json!({"object": "block"}))
        .unwrap();
    assert!(approx_relative(
        m.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0
    ));

    // --- STEP export → artifact → reimport ------------------------------------
    let ex = engine
        .execute("cad.export_step", json!({"object": "block"}))
        .unwrap();
    let step_ref = ex.output["step"].as_str().unwrap().to_string();
    let step_bytes = blob_bytes(&art_dir, &step_ref);
    let step_text = String::from_utf8_lossy(&step_bytes);
    assert!(step_text.contains("ISO-10303-21"));

    // reimport through a fresh kernel and re-measure — real round-trip
    let kernel2 = CadrumKernel::new();
    let reimported = kernel2.import_step(&step_bytes).unwrap();
    let rm = kernel2.measure(reimported).unwrap();
    assert!(approx_relative(rm.volume_mm3, 40_000.0));

    // --- undo / redo ----------------------------------------------------------
    // History is [create, measure, export_step]: three undos reach the
    // create; three redos restore every derived write in order.
    engine.undo().unwrap();
    engine.undo().unwrap();
    engine.undo().unwrap();
    assert!(engine.project().find_by_name("block").is_none());
    engine.redo().unwrap();
    engine.redo().unwrap();
    engine.redo().unwrap();
    let block = engine.project().find_by_name("block").unwrap();
    let shape_state = block
        .component_data(worldos_kernel::known::components::CAD_SHAPE)
        .unwrap();
    assert_eq!(shape_state["brep"].as_str().unwrap(), brep_ref);
    assert_eq!(shape_state["step"].as_str().unwrap(), step_ref);

    // --- save -------------------------------------------------------------------
    engine.save().unwrap();

    // --- reopen + regenerate ----------------------------------------------------
    drop(engine);
    let mut engine2 = Engine::open(&path).unwrap();
    let block2 = engine2.project().find_by_name("block").unwrap();
    assert_eq!(block2.id.to_string(), id);

    let op = block2
        .component_data(worldos_kernel::known::components::CAD_OPERATION)
        .unwrap();
    assert_eq!(op["kind"].as_str().unwrap(), "create_box");
    assert_eq!(op["kernel"].as_str().unwrap(), "occt-8.0.1-cadrum");

    let state = block2
        .component_data(worldos_kernel::known::components::CAD_SHAPE)
        .unwrap();
    assert_eq!(state["brep"].as_str().unwrap(), brep_ref);
    assert_eq!(state["step"].as_str().unwrap(), step_ref);

    // regenerate from the persisted recipe params — parametric truth
    let params = &op["params"];
    let kernel3 = CadrumKernel::new();
    let size: Vec<f64> = params["size_mm"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let regen = kernel3.make_box(size[0], size[1], size[2]).unwrap();
    let regen_m = kernel3.measure(regen).unwrap();
    assert!(approx_relative(regen_m.volume_mm3, 40_000.0));

    // and through the semantic layer: fresh kernel attached, same commands
    engine2.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    let m2 = engine2
        .execute("cad.measure", json!({"object": "block"}))
        .unwrap();
    assert!(approx_relative(
        m2.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0
    ));
}

#[test]
fn cad_commands_fail_cleanly_without_kernel_or_for_wrong_objects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.worldos");

    let mut engine = Engine::create("plain", &path).unwrap();
    // no kernel attached → cad.* commands are unregistered
    assert!(
        engine
            .execute("cad.create_box", json!({"size_mm": 10.0}))
            .is_err()
    );

    engine.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    // a non-cad object cannot be measured as a body
    engine
        .execute(
            "object.create",
            json!({"type": "core:note", "name": "memo", "components": {}}),
        )
        .unwrap();
    let err = engine
        .execute("cad.measure", json!({"object": "memo"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a cad:body"), "unexpected: {err}");

    // malformed inputs fail schema validation, not the kernel
    assert!(
        engine
            .execute("cad.create_box", json!({"size_mm": "big"}))
            .is_err()
    );
    assert!(
        engine
            .execute("cad.create_sphere", json!({"radius_mm": -2.0}))
            .is_err()
    );

    // artifact store: sidecar created next to the file
    assert!(sidecar(&path).is_dir());
}

#[test]
fn artifacts_follow_save_as() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("a.worldos");
    let p2 = dir.path().join("b.worldos");

    let mut engine = Engine::create("migr", &p1).unwrap();
    engine.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    let out = engine
        .execute("cad.create_box", json!({"size_mm": 10.0, "name": "b"}))
        .unwrap();
    let brep_ref = out.output["brep"].as_str().unwrap().to_string();
    assert!(sidecar(&p1).is_dir());

    // save to a NEW location → blobs migrate to the new sidecar
    engine.save_as(&p2).unwrap();
    let new_blob = sidecar(&p2)
        .join("objects")
        .join(&brep_ref["sha256:".len()..][..2])
        .join(&brep_ref["sha256:".len()..]);
    assert!(new_blob.is_file(), "blob did not migrate to {new_blob:?}");

    // reopen the moved file — everything still resolves
    drop(engine);
    let mut engine3 = Engine::open(&p2).unwrap();
    engine3.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    let m = engine3
        .execute("cad.measure", json!({"object": "b"}))
        .unwrap();
    assert!(approx_relative(
        m.output["measures"]["volume_mm3"].as_f64().unwrap(),
        1000.0
    ));
}

#[test]
fn feature_chain_boolean_fillet_chamfer_transform_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chain.worldos");

    let mut engine = Engine::create("chain", &path).unwrap();
    engine.attach_cad(Arc::new(CadrumKernel::new())).unwrap();

    // block 50x40x20 (40_000 mm3) with a centered cylinder tool
    let block = engine
        .execute(
            "cad.create_box",
            json!({"size_mm": [50.0, 40.0, 20.0], "name": "block"}),
        )
        .unwrap();
    let block_id = block.output["id"].as_str().unwrap().to_string();
    engine
        .execute(
            "cad.create_cylinder",
            json!({"radius_mm": 5.0, "height_mm": 30.0, "name": "tool",
                   "position": [25.0, 20.0, -5.0]}),
        )
        .unwrap();

    // boolean subtract -> hole through the block
    let cut = engine
        .execute(
            "cad.boolean",
            json!({"a": "block", "b": "tool", "op": "subtract", "name": "plate"}),
        )
        .unwrap();
    let plate_id = cut.output["id"].as_str().unwrap().to_string();
    let hole = std::f64::consts::PI * 25.0 * 20.0; // pi r^2 h through 20 mm
    assert!(approx_relative(
        cut.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0 - hole
    ));
    assert!(cut.output["topology"]["is_valid"].as_bool().unwrap());

    // derived-from edges exist (feature-tree lineage)
    let derived: Vec<String> = engine
        .project()
        .relations_from(plate_id.parse().unwrap())
        .filter(|r| r.type_id == worldos_kernel::known::rel::DERIVED_FROM)
        .map(|r| r.to.to_string())
        .collect();
    assert_eq!(derived.len(), 2);
    assert!(derived.contains(&block_id));

    // union puts material back (block fused with a boss)
    let union = engine
        .execute(
            "cad.boolean",
            json!({"a": "block", "b": "tool", "op": "union"}),
        )
        .unwrap();
    assert!(union.output["measures"]["volume_mm3"].as_f64().unwrap() > 40_000.0);

    // fillet + chamfer reduce volume and stay valid solids
    let fillet = engine
        .execute(
            "cad.fillet",
            json!({"object": "plate", "radius_mm": 1.0, "name": "plate_f"}),
        )
        .unwrap();
    let fv = fillet.output["measures"]["volume_mm3"].as_f64().unwrap();
    assert!(fv < 40_000.0 - hole + 1.0);
    assert!(fillet.output["topology"]["is_valid"].as_bool().unwrap());

    let chamfer = engine
        .execute(
            "cad.chamfer",
            json!({"object": "plate", "distance_mm": 0.8}),
        )
        .unwrap();
    assert!(chamfer.output["topology"]["is_valid"].as_bool().unwrap());

    // transform: translate + uniform scale -> volume scales by factor^3
    let moved = engine
        .execute(
            "cad.transform",
            json!({"object": "block",
                   "ops": [
                       {"kind": "translate", "delta_mm": [100.0, 0.0, 0.0]},
                       {"kind": "scale", "center_mm": [0.0, 0.0, 0.0], "factor": 2.0}
                   ],
                   "name": "block2x"}),
        )
        .unwrap();
    assert!(approx_relative(
        moved.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0 * 8.0
    ));
    let bbox = &moved.output["measures"]["bbox"];
    assert!(bbox["min_mm"][0].as_f64().unwrap() > 150.0); // translated+scaled

    // STEP round-trip through the semantic layer: export plate, reimport
    let ex = engine
        .execute("cad.export_step", json!({"object": "plate"}))
        .unwrap();
    let step_ref = ex.output["step"].as_str().unwrap();
    let imp = engine
        .execute(
            "cad.import_step",
            json!({"step": step_ref, "name": "plate_re"}),
        )
        .unwrap();
    assert!(approx_relative(
        imp.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0 - hole
    ));
    assert!(imp.output["topology"]["is_valid"].as_bool().unwrap());

    // every derived object is undoable. History tail is
    // [.., transform, export_step, import_step] so undoing walks it back.
    engine.undo().unwrap(); // import_step
    assert!(engine.project().find_by_name("plate_re").is_none());
    engine.undo().unwrap(); // export_step (step ref on plate)
    engine.undo().unwrap(); // transform
    assert!(engine.project().find_by_name("block2x").is_none());
    engine.undo().unwrap(); // chamfer
    engine.undo().unwrap(); // fillet
    assert!(engine.project().find_by_name("plate_f").is_none());
    // redo one step back
    engine.redo().unwrap();
    assert!(engine.project().find_by_name("plate_f").is_some());

    // bad inputs fail at schema/feature level, never panic
    assert!(
        engine
            .execute(
                "cad.boolean",
                json!({"a": "block", "b": "tool", "op": "merge"})
            )
            .is_err()
    );
    assert!(
        engine
            .execute("cad.fillet", json!({"object": "block", "radius_mm": -1.0}))
            .is_err()
    );
    assert!(
        engine
            .execute("cad.transform", json!({"object": "block", "ops": []}))
            .is_err()
    );
}
