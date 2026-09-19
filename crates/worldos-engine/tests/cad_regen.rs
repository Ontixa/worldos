//! Multi-level parametric regeneration proofs:
//!
//!   A → B → C chains and branched dependency graphs, deterministic
//!   topological cascades, cycle rejection, stale-upstream rejection,
//!   stale topology-selection rejection, atomic mid-chain failure,
//!   undo/redo across derived ops.
//!
//! All mutation goes through `Engine::execute` — the governed path CLI,
//! RPC, MCP, SDKs and agents share.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use worldos_adapter_cadrum::CadrumKernel;
use worldos_cad::approx_relative;
use worldos_engine::Engine;

fn setup(dir: &Path, name: &str) -> Engine {
    let mut e = Engine::create(name, dir.join(format!("{name}.worldos"))).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    e
}

fn shape(e: &Engine, name: &str) -> Value {
    e.project()
        .find_by_name(name)
        .unwrap()
        .component_data(worldos_kernel::known::components::CAD_SHAPE)
        .unwrap()
        .clone()
}

fn recipe(e: &Engine, name: &str) -> Value {
    e.project()
        .find_by_name(name)
        .unwrap()
        .component_data(worldos_kernel::known::components::CAD_OPERATION)
        .unwrap()
        .clone()
}

fn stale(e: &Engine, name: &str) -> bool {
    shape(e, name)["stale"].as_bool().unwrap_or(false)
}

/// A → B → C: base block, subtract hole, chamfer the plate.
fn build_chain(e: &mut Engine) {
    e.execute(
        "cad.create_box",
        json!({"size_mm": [50.0, 40.0, 20.0], "name": "base"}),
    )
    .unwrap();
    e.execute(
        "cad.create_cylinder",
        json!({"radius_mm": 5.0, "height_mm": 30.0,
               "position": [25.0, 20.0, -5.0], "name": "tool"}),
    )
    .unwrap();
    e.execute(
        "cad.boolean",
        json!({"a": "base", "b": "tool", "op": "subtract", "name": "plate"}),
    )
    .unwrap();
    e.execute(
        "cad.chamfer",
        json!({"object": "plate", "distance_mm": 0.5, "name": "plate_c"}),
    )
    .unwrap();
}

#[test]
fn transitive_staleness_marks_every_dependent() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "trans");

    build_chain(&mut e);
    assert!(!stale(&e, "plate"));
    assert!(!stale(&e, "plate_c"));

    // Head change: BOTH the direct dependent AND its dependent stale.
    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [80.0, 40.0, 20.0]}}),
    )
    .unwrap();
    assert!(stale(&e, "plate"), "direct dependent not marked stale");
    assert!(stale(&e, "plate_c"), "transitive dependent not marked stale");
}

#[test]
fn cascade_regenerates_in_topological_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "casc");
    build_chain(&mut e);

    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [80.0, 40.0, 20.0]}}),
    )
    .unwrap();

    // Single-object regen refuses while an upstream is stale.
    let err = e
        .execute("cad.regenerate", json!({"object": "plate_c"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("stale upstream"), "unexpected: {err}");
    assert!(stale(&e, "plate_c"));

    // Cascade from the head: plate first, then plate_c — and both fresh.
    let out = e
        .execute(
            "cad.regenerate",
            json!({"object": "plate", "cascade": true}),
        )
        .unwrap();
    let names: Vec<&str> = out.output["regenerated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["plate", "plate_c"]);
    let hole = std::f64::consts::PI * 25.0 * 20.0;
    let v = out.output["regenerated"][0]["measures"]["volume_mm3"]
        .as_f64()
        .unwrap();
    // 80×40×20 = 64_000 mm³ minus the r=5 hole through 20 mm.
    assert!(
        approx_relative(v, 64_000.0 - hole),
        "plate volume {v} != {}",
        64_000.0 - hole
    );
    assert!(!stale(&e, "plate"));
    assert!(!stale(&e, "plate_c"));

    // all_stale is a no-op when nothing is stale.
    let out = e.execute("cad.regenerate", json!({"all_stale": true})).unwrap();
    assert_eq!(out.output["count"].as_u64().unwrap(), 0);
}

#[test]
fn branched_dependencies_regen_all_stale() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "branch");

    // base → {filleted, chamfered} → union joins the branches.
    e.execute(
        "cad.create_box",
        json!({"size_mm": [40.0, 40.0, 40.0], "name": "base"}),
    )
    .unwrap();
    e.execute(
        "cad.fillet",
        json!({"object": "base", "radius_mm": 2.0, "name": "f"}),
    )
    .unwrap();
    e.execute(
        "cad.chamfer",
        json!({"object": "base", "distance_mm": 2.0, "name": "c"}),
    )
    .unwrap();
    // A second source so the union has two distinct inputs.
    e.execute(
        "cad.create_box",
        json!({"size_mm": [10.0, 10.0, 60.0], "name": "pin",
               "position": [15.0, 15.0, -10.0]}),
    )
    .unwrap();
    e.execute(
        "cad.boolean",
        json!({"a": "f", "b": "pin", "op": "union", "name": "assy"}),
    )
    .unwrap();

    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [60.0, 40.0, 40.0]}}),
    )
    .unwrap();
    assert!(stale(&e, "f") && stale(&e, "c") && stale(&e, "assy"));

    // all_stale covers the whole diamond in one transaction.
    let out = e.execute("cad.regenerate", json!({"all_stale": true})).unwrap();
    assert_eq!(out.output["count"].as_u64().unwrap(), 3);
    assert!(!stale(&e, "f") && !stale(&e, "c") && !stale(&e, "assy"));
    let assy = e
        .execute("cad.measure", json!({"object": "assy"}))
        .unwrap();
    // 60×40×40 box filleted + pin union: volume > the unmodified box
    // filleted alone; exact check unnecessary — validity + freshness is.
    assert!(assy.output["topology"]["is_valid"].as_bool().unwrap());
}

#[test]
fn mid_chain_failure_rolls_back_whole_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "atomic");
    build_chain(&mut e);

    // Poison plate_c's recipe: an edge id that cannot exist → the
    // stale-selection guard fails deterministically on regen.
    let mut op = recipe(&e, "plate_c");
    op["params"]["edge_ids"] = json!([4_242_424_242u64]);
    e.execute(
        "object.set_component",
        json!({"name": "plate_c", "component": "cad:operation", "data": op}),
    )
    .unwrap();

    let before_plate = shape(&e, "plate")["brep"].as_str().unwrap().to_string();
    let before_pc = shape(&e, "plate_c")["brep"].as_str().unwrap().to_string();

    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [80.0, 40.0, 20.0]}}),
    )
    .unwrap();

    // Cascade: plate regens fine, plate_c fails → whole command rolls
    // back. No half-new, half-old chain may be left reporting fresh.
    let err = e
        .execute("cad.regenerate", json!({"object": "plate", "cascade": true}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("stale edge selection"), "unexpected: {err}");

    assert!(stale(&e, "plate"), "rolled-back plate must stay stale");
    assert!(stale(&e, "plate_c"));
    assert_eq!(shape(&e, "plate")["brep"].as_str().unwrap(), before_plate);
    assert_eq!(shape(&e, "plate_c")["brep"].as_str().unwrap(), before_pc);

    // The healthy prefix can still be regenerated alone afterwards.
    e.execute("cad.regenerate", json!({"object": "plate"}))
        .unwrap();
    assert!(!stale(&e, "plate"));
    assert!(stale(&e, "plate_c"), "leaf remains honestly stale");
}

#[test]
fn stale_edge_selection_is_rejected_not_rechosen() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "edgesel");

    e.execute(
        "cad.create_box",
        json!({"size_mm": [30.0, 30.0, 30.0], "name": "b"}),
    )
    .unwrap();
    // Fillet a SPECIFIC edge, so the recipe records a selection.
    let topo = e.execute("cad.measure", json!({"object": "b"})).unwrap();
    let edge = topo.output["topology"]["edge_ids"][0].as_u64().unwrap();
    e.execute(
        "cad.fillet",
        json!({"object": "b", "radius_mm": 1.0, "edge_ids": [edge], "name": "f"}),
    )
    .unwrap();

    // Corrupt the recipe's selection — regeneration must fail loudly
    // instead of silently picking a different edge by ordinal.
    let mut op = recipe(&e, "f");
    op["params"]["edge_ids"] = json!([9_999_999_999u64]);
    e.execute(
        "object.set_component",
        json!({"name": "f", "component": "cad:operation", "data": op}),
    )
    .unwrap();

    e.execute(
        "cad.set_param",
        json!({"object": "b", "params": {"size_mm": [40.0, 30.0, 30.0]}}),
    )
    .unwrap();
    let err = e
        .execute("cad.regenerate", json!({"object": "f"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("stale edge selection"), "unexpected: {err}");
    assert!(stale(&e, "f"), "object must stay stale, not silently fixed");
}

#[test]
fn retarget_rejects_cycles_and_updates_edges() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "cycle");

    e.execute(
        "cad.create_box",
        json!({"size_mm": [20.0, 20.0, 20.0], "name": "a"}),
    )
    .unwrap();
    e.execute(
        "cad.create_box",
        json!({"size_mm": [10.0, 10.0, 10.0], "name": "b"}),
    )
    .unwrap();
    e.execute(
        "cad.boolean",
        json!({"a": "a", "b": "b", "op": "union", "name": "u"}),
    )
    .unwrap();
    e.execute(
        "cad.chamfer",
        json!({"object": "u", "distance_mm": 0.5, "name": "u_c"}),
    )
    .unwrap();

    // Retarget u_c's source to u_c itself → cycle, rejected.
    let err = e
        .execute(
            "cad.set_param",
            json!({"object": "u_c",
                   "params": {"source": e.project().find_by_name("u_c").unwrap().id.to_string()}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("cycle"), "unexpected: {err}");

    // Retarget union's `a` to its own dependent → cycle, rejected.
    let err = e
        .execute(
            "cad.set_param",
            json!({"object": "u",
                   "params": {"a": e.project().find_by_name("u_c").unwrap().id.to_string()}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("cycle"), "unexpected: {err}");

    // Nothing changed — recipe params and edges intact.
    let u = recipe(&e, "u");
    let a_id = e.project().find_by_name("a").unwrap().id.to_string();
    assert_eq!(u["params"]["a"].as_str().unwrap(), a_id);

    // Legal retarget: union.a → a fresh box works and resyncs edges.
    e.execute(
        "cad.create_box",
        json!({"size_mm": [30.0, 20.0, 20.0], "name": "a2"}),
    )
    .unwrap();
    let a2_id = e.project().find_by_name("a2").unwrap().id;
    e.execute(
        "cad.set_param",
        json!({"object": "u", "params": {"a": a2_id.to_string()}}),
    )
    .unwrap();
    let edges: Vec<String> = e
        .project()
        .relations_from(e.project().find_by_name("u").unwrap().id)
        .filter(|r| r.type_id == worldos_kernel::known::rel::DERIVED_FROM)
        .map(|r| r.to.to_string())
        .collect();
    assert!(edges.contains(&a2_id.to_string()));
    assert!(!edges.contains(&a_id));
}

#[test]
fn undo_redo_across_cascade_restores_stale_flags() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "undo");
    build_chain(&mut e);

    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [80.0, 40.0, 20.0]}}),
    )
    .unwrap();
    e.execute("cad.regenerate", json!({"all_stale": true}))
        .unwrap();
    assert!(!stale(&e, "plate") && !stale(&e, "plate_c"));

    // Undo the cascade — stale flags come back.
    e.undo().unwrap();
    assert!(stale(&e, "plate") && stale(&e, "plate_c"));
    // Redo — fresh again.
    e.redo().unwrap();
    assert!(!stale(&e, "plate") && !stale(&e, "plate_c"));

    // Undo back past the regenerate, then the set_param: original
    // geometry restored everywhere.
    e.undo().unwrap(); // regenerate
    assert!(stale(&e, "plate") && stale(&e, "plate_c"));
    e.undo().unwrap(); // set_param
    let m = e.execute("cad.measure", json!({"object": "base"})).unwrap();
    assert!(approx_relative(
        m.output["measures"]["volume_mm3"].as_f64().unwrap(),
        40_000.0
    ));
}

#[test]
fn reopen_preserves_chain_and_regens_transitively() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.worldos");

    let mut e = Engine::create("reopen", &path).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    build_chain(&mut e);
    e.execute(
        "cad.set_param",
        json!({"object": "base", "params": {"size_mm": [80.0, 40.0, 20.0]}}),
    )
    .unwrap();
    e.save().unwrap();
    drop(e);

    // Stale flags and recipes survive the round-trip.
    let mut e2 = Engine::open(&path).unwrap();
    e2.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    assert!(stale(&e2, "plate") && stale(&e2, "plate_c"));
    let out = e2.execute("cad.regenerate", json!({"all_stale": true})).unwrap();
    assert_eq!(out.output["count"].as_u64().unwrap(), 2);
    assert!(!stale(&e2, "plate") && !stale(&e2, "plate_c"));
}

#[test]
fn invalid_param_rolls_back_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(dir.path(), "badparam");
    build_chain(&mut e);

    let before = recipe(&e, "base")["params"]["size_mm"].clone();
    let before_brep = shape(&e, "base")["brep"].as_str().unwrap().to_string();

    // Negative dimension: kernel rejects → recipe and shape untouched,
    // no dependent marked.
    assert!(
        e.execute(
            "cad.set_param",
            json!({"object": "base", "params": {"size_mm": [-5.0, 40.0, 20.0]}}),
        )
        .is_err()
    );
    assert_eq!(recipe(&e, "base")["params"]["size_mm"], before);
    assert_eq!(shape(&e, "base")["brep"].as_str().unwrap(), before_brep);
    assert!(!stale(&e, "plate") && !stale(&e, "plate_c"));

    // Non-finite input rejected by schema validation before the kernel.
    assert!(
        e.execute(
            "cad.set_param",
            json!({"object": "base", "params": {"size_mm": [f64::NAN, 40.0, 20.0]}}),
        )
        .is_err()
    );
}
