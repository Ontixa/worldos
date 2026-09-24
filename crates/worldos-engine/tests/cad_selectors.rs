//! Semantic topology selectors through the governed command path:
//! `cad.select` probes, `edge_select` on `cad.fillet`/`cad.chamfer`,
//! `select` on `cad.measure`, and selector replay across parametric
//! regeneration (`cad.set_param` → `cad.regenerate`).
//!
//! Everything goes through `Engine::execute` — the same path CLI, RPC,
//! MCP, SDKs and agents use. Selectors never bypass the command layer.

use std::sync::Arc;

use serde_json::json;
use worldos_adapter_cadrum::CadrumKernel;
use worldos_cad::{Selector, approx_mm, approx_relative, resolve_edges, resolve_faces};
use worldos_engine::Engine;

fn engine_with_cad(dir: &tempfile::TempDir) -> Engine {
    let path = dir.path().join("sel.worldos");
    let mut engine = Engine::create("sel", &path).unwrap();
    engine.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    engine
}

fn make_box(engine: &mut Engine, size: [f64; 3], name: &str) {
    engine
        .execute("cad.create_box", json!({"size_mm": size, "name": name}))
        .unwrap();
}

#[test]
fn cad_select_resolves_faces_and_edges() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [50.0, 40.0, 20.0], "block");

    // top_face → exactly one face, outward normal +Z
    let out = engine
        .execute(
            "cad.select",
            json!({"object": "block", "select": {"op": "top_face"}}),
        )
        .unwrap();
    assert_eq!(out.output["kind"], "faces");
    assert_eq!(out.output["matched"], 1);
    let face = &out.output["faces"][0];
    assert!(approx_mm(face["normal"][2].as_f64().unwrap(), 1.0));
    assert!(approx_mm(face["center_mm"][2].as_f64().unwrap(), 20.0));
    assert_eq!(face["surface"], "plane");

    // bare-string shorthand
    let out2 = engine
        .execute(
            "cad.select",
            json!({"object": "block", "select": "bottom_face"}),
        )
        .unwrap();
    assert_eq!(out2.output["matched"], 1);
    assert!(approx_mm(
        out2.output["faces"][0]["normal"][2].as_f64().unwrap(),
        -1.0
    ));

    // edges_adjacent_to(top_face) → the 4 top edges, with detail
    let out = engine
        .execute(
            "cad.select",
            json!({"object": "block",
                   "select": {"op": "edges_adjacent_to",
                              "faces": {"op": "top_face"}}}),
        )
        .unwrap();
    assert_eq!(out.output["kind"], "edges");
    assert_eq!(out.output["matched"], 4);
    assert_eq!(out.output["edges"].as_array().unwrap().len(), 4);
    for e in out.output["edges"].as_array().unwrap() {
        // every matched edge is horizontal at z=20
        assert!(approx_mm(e["start_mm"][2].as_f64().unwrap(), 20.0));
        assert!(approx_mm(e["end_mm"][2].as_f64().unwrap(), 20.0));
    }

    // edges_extreme(+Z) selects the same 4 edges on a box
    let out = engine
        .execute(
            "cad.select",
            json!({"object": "block",
                   "select": {"op": "edges_extreme", "dir": [0, 0, 1]}}),
        )
        .unwrap();
    assert_eq!(out.output["matched"], 4);

    // an empty match is a report, not an error
    let out = engine
        .execute(
            "cad.select",
            json!({"object": "block",
                   "select": {"op": "faces_of_kind", "kind": "torus"}}),
        )
        .unwrap();
    assert_eq!(out.output["matched"], 0);
}

#[test]
fn cad_measure_reports_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [50.0, 40.0, 20.0], "block");

    let out = engine
        .execute(
            "cad.measure",
            json!({"object": "block",
                   "select": {"op": "faces_normal_to", "dir": [0, 0, 1]}}),
        )
        .unwrap();
    assert_eq!(out.output["selection"]["kind"], "faces");
    assert_eq!(out.output["selection"]["matched"], 1);
    // top face of a 50×40 plan is 2000 mm²
    assert!(approx_mm(
        out.output["selection"]["area_mm2"].as_f64().unwrap(),
        2000.0
    ));
    // union of top+bottom → 2 faces, 4000 mm²
    let out = engine
        .execute(
            "cad.measure",
            json!({"object": "block",
                   "select": {"op": "union",
                              "of": [{"op": "top_face"}, {"op": "bottom_face"}]}}),
        )
        .unwrap();
    assert_eq!(out.output["selection"]["matched"], 2);
    assert!(approx_mm(
        out.output["selection"]["area_mm2"].as_f64().unwrap(),
        4000.0
    ));
}

#[test]
fn fillet_with_edge_select_persists_selector_in_recipe() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [50.0, 40.0, 20.0], "block");

    let out = engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 2.0, "name": "rounded",
                   "edge_select": {"op": "edges_adjacent_to",
                                   "faces": {"op": "top_face"}}}),
        )
        .unwrap();
    let vol = out.output["measures"]["volume_mm3"].as_f64().unwrap();
    assert!(vol < 40_000.0 && vol > 39_000.0, "fillet vol {vol}");
    assert!(out.output["topology"]["is_valid"].as_bool().unwrap());

    // the recipe stores the SELECTOR, not resolved ids — that's what
    // makes replay stable
    let rounded = engine.project().find_by_name("rounded").unwrap();
    let op = rounded
        .component_data(worldos_kernel::known::components::CAD_OPERATION)
        .unwrap();
    assert_eq!(op["params"]["edge_select"]["op"], "edges_adjacent_to");
    assert_eq!(op["params"]["edge_select"]["faces"]["op"], "top_face");
    assert!(op["params"]["edge_ids"].is_null() || op["params"].get("edge_ids").is_none());
}

#[test]
fn selector_recipe_survives_parametric_regeneration() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [50.0, 40.0, 20.0], "block");

    engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 2.0, "name": "rounded",
                   "edge_select": {"op": "edges_adjacent_to",
                                   "faces": {"op": "top_face"}}}),
        )
        .unwrap();

    // parametric edit on the source — rounded goes stale
    engine
        .execute(
            "cad.set_param",
            json!({"object": "block", "params": {"size_mm": [60.0, 40.0, 20.0]}}),
        )
        .unwrap();
    let stale = engine
        .project()
        .find_by_name("rounded")
        .unwrap()
        .component_data(worldos_kernel::known::components::CAD_SHAPE)
        .unwrap()["stale"]
        .as_bool()
        .unwrap_or(false);
    assert!(stale, "derived body must flag stale after source regen");

    // replay: the selector re-resolves on the NEW 60-mm box topology
    let out = engine
        .execute("cad.regenerate", json!({"object": "rounded"}))
        .unwrap();

    // ground truth: same selector on a fresh 60×40×20 box via the kernel
    let kernel = engine.cad().unwrap().kernel.clone();
    let raw = kernel.make_box(60.0, 40.0, 20.0).unwrap();
    let view = kernel.topology_view(raw).unwrap();
    let top = resolve_faces(&Selector::TopFace, &view).unwrap();
    let top_edges = resolve_edges(
        &Selector::EdgesAdjacentTo {
            faces: Box::new(Selector::FaceIds {
                ids: top.into_iter().collect(),
            }),
        },
        &view,
    )
    .unwrap();
    let ids: Vec<u64> = top_edges.into_iter().collect();
    assert_eq!(ids.len(), 4);
    let expected = kernel.fillet(raw, 2.0, &ids).unwrap();
    let expected_vol = kernel.measure(expected).unwrap().volume_mm3;

    assert!(
        approx_relative(
            out.output["measures"]["volume_mm3"].as_f64().unwrap(),
            expected_vol
        ),
        "regen volume must match a direct fillet of the new top edges"
    );
    // and it must actually differ from the pre-edit body
    let vol = out.output["measures"]["volume_mm3"].as_f64().unwrap();
    assert!(vol > 40_000.0 && vol < 48_000.0);
}

#[test]
fn selector_failure_paths_are_structured_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [40.0, 40.0, 20.0], "block"); // square plan
    engine
        .execute(
            "cad.create_sphere",
            json!({"radius_mm": 10.0, "name": "ball"}),
        )
        .unwrap();

    // ambiguous: +X and +Y faces tie along the [1,1,0] diagonal
    let err = engine
        .execute(
            "cad.select",
            json!({"object": "block",
                   "select": {"op": "face_extreme", "dir": [1, 1, 0]}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("ambiguous"), "unexpected: {err}");

    // empty: a sphere has no planar +Z face, so its edges_adjacent_to
    // resolves to nothing — a fillet cannot proceed
    let err = engine
        .execute(
            "cad.fillet",
            json!({"object": "ball", "radius_mm": 1.0,
                   "edge_select": {"op": "edges_adjacent_to",
                                   "faces": {"op": "top_face"}}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("matched no edges"), "unexpected: {err}");

    // stale reference: a raw edge id that does not exist on the shape
    let err = engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 1.0, "edge_ids": [999999]}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not present"), "unexpected: {err}");

    // kind mismatch: a face selector where edges are required
    let err = engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 1.0,
                   "edge_select": {"op": "top_face"}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("targets faces"), "unexpected: {err}");

    // malformed selector payloads
    assert!(
        engine
            .execute(
                "cad.select",
                json!({"object": "block", "select": {"op": "nope"}})
            )
            .is_err()
    );
    assert!(
        engine
            .execute(
                "cad.select",
                json!({"object": "block",
                       "select": {"op": "faces_normal_to", "dir": [0, 0, 0]}})
            )
            .is_err()
    );
    assert!(
        engine
            .execute(
                "cad.select",
                json!({"object": "block",
                       "select": {"op": "union",
                                  "of": [{"op": "top_face"}, {"op": "all_edges"}]}})
            )
            .is_err()
    );

    // none of the failures touched the graph
    let block = engine.project().find_by_name("block").unwrap();
    let shape = block
        .component_data(worldos_kernel::known::components::CAD_SHAPE)
        .unwrap();
    assert!(approx_relative(
        shape["measures"]["volume_mm3"].as_f64().unwrap(),
        32_000.0
    ));
}

#[test]
fn stale_raw_ids_in_a_recipe_fail_the_replay_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_with_cad(&dir);
    make_box(&mut engine, [50.0, 40.0, 20.0], "block");

    // Kernel topology ids are load-scoped (cadrum reports TShape
    // addresses): every command re-imports the BRep, so an id captured
    // by `cad.measure` is already dead when `cad.fillet` runs — a raw
    // `edge_ids` input fails `SelectorStale` honestly instead of
    // silently matching nothing (or worse, an unrelated edge whose
    // TShape reused the address). u64::MAX is never a live address.
    let err = engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 1.0,
                   "edge_ids": [u64::MAX], "name": "f"}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not present"), "unexpected: {err}");
    assert!(engine.project().find_by_name("f").is_none());

    // A recipe that somehow carries raw ids (e.g. merged in through
    // set_param) must fail the replay the same way — fail closed, no
    // half-applied state, no silent no-op fillet.
    engine
        .execute(
            "cad.fillet",
            json!({"object": "block", "radius_mm": 1.0, "name": "f",
                   "edge_select": {"op": "edges_adjacent_to",
                                   "faces": {"op": "top_face"}}}),
        )
        .unwrap();
    let err = engine
        .execute(
            "cad.set_param",
            json!({"object": "f", "params": {"edge_ids": [u64::MAX]}}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not present"), "unexpected: {err}");

    // The failed set_param did not pollute the stored recipe — the
    // selector replay still works.
    let out = engine
        .execute("cad.regenerate", json!({"object": "f"}))
        .unwrap();
    assert!(out.output["topology"]["is_valid"].as_bool().unwrap());
}
