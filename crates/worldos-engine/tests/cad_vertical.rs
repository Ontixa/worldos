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
    assert_eq!(out.output["topology"]["is_valid"].as_bool().unwrap(), true);

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
