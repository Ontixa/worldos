#![no_main]

//! Fuzz the CAD selector boundary: `Selector` expressions arrive as
//! untrusted JSON — RPC `select`/`edge_select` params and persisted
//! `cad:operation` recipes re-resolved on `cad.regenerate` — and are
//! resolved against kernel-reported `TopologyView` data. Resolution is
//! pure data-in/data-out: it must never panic, overflow, or recurse
//! without bound on hostile expressions or hostile views.
//!
//! Two decodes per input, mirroring the real seams:
//! 1. the command-layer `parse_selector` shape — a JSON object or a
//!    bare string shorthand — resolved against a fixed box-like view;
//! 2. a `[selector, view]` JSON pair — the view is kernel-reported
//!    data, but it is still plain input to the resolver.

use libfuzzer_sys::fuzz_target;
use worldos_cad::{EdgeDetail, FaceDetail, FaceSurface, Selector, TopologyView};

/// Small deterministic box-ish view: mixed planar/curved faces plus a
/// second upward planar face at the same z-center, so `top_face` can
/// reach the ambiguous-tie path.
fn box_view() -> TopologyView {
    let face = |id: u64,
                center: [f64; 3],
                normal: Option<[f64; 3]>,
                axis: Option<[f64; 3]>,
                surface: FaceSurface,
                edge_ids: Vec<u64>,
                area: f64| FaceDetail {
        id,
        center_mm: center,
        normal,
        axis,
        surface,
        edge_ids,
        area_mm2: area,
    };
    let edge = |id: u64, start: [f64; 3], end: [f64; 3]| EdgeDetail {
        id,
        start_mm: start,
        end_mm: end,
    };
    TopologyView {
        faces: vec![
            face(
                1,
                [0.0, 0.0, 10.0],
                Some([0.0, 0.0, 1.0]),
                Some([0.0, 0.0, 1.0]),
                FaceSurface::Plane,
                vec![1, 2, 3, 4],
                100.0,
            ),
            face(
                2,
                [0.0, 0.0, 0.0],
                Some([0.0, 0.0, -1.0]),
                Some([0.0, 0.0, -1.0]),
                FaceSurface::Plane,
                vec![5, 6, 7, 8],
                100.0,
            ),
            face(
                3,
                [5.0, 0.0, 5.0],
                Some([1.0, 0.0, 0.0]),
                Some([1.0, 0.0, 0.0]),
                FaceSurface::Plane,
                vec![1, 5],
                50.0,
            ),
            face(
                4,
                [-5.0, 0.0, 5.0],
                Some([-1.0, 0.0, 0.0]),
                Some([-1.0, 0.0, 0.0]),
                FaceSurface::Plane,
                vec![2, 6],
                50.0,
            ),
            // Same outward +Z and z-center as face 1 — `top_face` ties.
            face(
                5,
                [30.0, 0.0, 10.0],
                Some([0.0, 0.0, 1.0]),
                Some([0.0, 0.0, 1.0]),
                FaceSurface::Plane,
                vec![9],
                25.0,
            ),
            // Curved faces report no single normal; axis stays.
            face(
                6,
                [0.0, 20.0, 5.0],
                None,
                Some([0.0, 0.0, 1.0]),
                FaceSurface::Cylinder,
                vec![10, 11],
                60.0,
            ),
            face(
                7,
                [0.0, -20.0, 5.0],
                None,
                None,
                FaceSurface::Sphere,
                vec![12],
                30.0,
            ),
        ],
        edges: vec![
            edge(1, [-5.0, -5.0, 10.0], [5.0, -5.0, 10.0]),
            edge(2, [-5.0, 5.0, 10.0], [-5.0, -5.0, 10.0]),
            edge(3, [5.0, 5.0, 10.0], [-5.0, 5.0, 10.0]),
            edge(4, [5.0, -5.0, 10.0], [5.0, 5.0, 10.0]),
            edge(5, [5.0, -5.0, 0.0], [5.0, -5.0, 10.0]),
            edge(6, [-5.0, 5.0, 0.0], [-5.0, 5.0, 10.0]),
            // Closed edge: start == end, like a seam or full circle.
            edge(10, [0.0, 20.0, 10.0], [0.0, 20.0, 10.0]),
            edge(11, [0.0, 20.0, 0.0], [10.0, 20.0, 0.0]),
            edge(12, [0.0, -20.0, 0.0], [0.0, -20.0, 10.0]),
        ],
    }
}

/// Every public entry point over `sel`/`view` — errors are values.
fn exercise(sel: &Selector, view: &TopologyView) {
    let _ = sel.target();
    let _ = sel.describe();
    let _ = worldos_cad::resolve(sel, view);
    let _ = worldos_cad::resolve_faces(sel, view);
    let _ = worldos_cad::resolve_edges(sel, view);
}

fuzz_target!(|data: &[u8]| {
    // Seam 1: command params / recipe fields. Bare strings are the
    // zero-arg shorthand (`"top_face"` ≡ `{"op":"top_face"}`), matching
    // the command layer's `parse_selector`.
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        let v = match v.as_str() {
            Some(op) => serde_json::json!({"op": op}),
            None => v,
        };
        if let Ok(sel) = serde_json::from_value::<Selector>(v) {
            exercise(&sel, &box_view());
        }
    }
    // Seam 2: `[selector, view]` — the view comes from the kernel, but
    // resolving over adversarial centers/normals/axes must still be
    // panic-free.
    if let Ok((sel, view)) = serde_json::from_slice::<(Selector, TopologyView)>(data) {
        exercise(&sel, &view);
    }
});
