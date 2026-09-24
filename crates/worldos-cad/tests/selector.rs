//! Selector resolution tests over hand-built `TopologyView`s — no
//! kernel involved (resolution is pure data). Kernel-level coverage
//! lives in `worldos-adapter-cadrum/tests/`, governed-path coverage in
//! `worldos-engine/tests/cad_selectors.rs`.

use serde_json::json;
use worldos_cad::error::CadError;
use worldos_cad::selector::{Selector, SelectorTarget, resolve, resolve_edges, resolve_faces};
use worldos_cad::types::{EdgeDetail, FaceDetail, FaceSurface, TopologyView};

fn face(
    id: u64,
    center_mm: [f64; 3],
    normal: Option<[f64; 3]>,
    axis: Option<[f64; 3]>,
    surface: FaceSurface,
    edge_ids: Vec<u64>,
) -> FaceDetail {
    FaceDetail {
        id,
        center_mm,
        normal,
        axis,
        surface,
        edge_ids,
        area_mm2: 1.0,
    }
}

fn edge(id: u64, start_mm: [f64; 3], end_mm: [f64; 3]) -> EdgeDetail {
    EdgeDetail {
        id,
        start_mm,
        end_mm,
    }
}

/// 40 × 40 × 20 box at origin: faces 1..6 = +X,-X,+Y,-Y,+Z,-Z; edges
/// 1..4 top (z=20), 5..8 bottom (z=0), 9..12 vertical (mid z=10).
fn box_view() -> TopologyView {
    let f = |id, c, n, e| face(id, c, Some(n), Some(n), FaceSurface::Plane, e);
    TopologyView {
        faces: vec![
            f(1, [40.0, 20.0, 10.0], [1.0, 0.0, 0.0], vec![1, 9, 5, 11]),
            f(2, [0.0, 20.0, 10.0], [-1.0, 0.0, 0.0], vec![3, 10, 7, 12]),
            f(3, [20.0, 40.0, 10.0], [0.0, 1.0, 0.0], vec![2, 12, 6, 9]),
            f(4, [20.0, 0.0, 10.0], [0.0, -1.0, 0.0], vec![4, 11, 8, 10]),
            f(5, [20.0, 20.0, 20.0], [0.0, 0.0, 1.0], vec![1, 2, 3, 4]),
            f(6, [20.0, 20.0, 0.0], [0.0, 0.0, -1.0], vec![5, 6, 7, 8]),
        ],
        edges: vec![
            edge(1, [40.0, 0.0, 20.0], [40.0, 40.0, 20.0]),
            edge(2, [40.0, 40.0, 20.0], [0.0, 40.0, 20.0]),
            edge(3, [0.0, 40.0, 20.0], [0.0, 0.0, 20.0]),
            edge(4, [0.0, 0.0, 20.0], [40.0, 0.0, 20.0]),
            edge(5, [40.0, 0.0, 0.0], [40.0, 40.0, 0.0]),
            edge(6, [40.0, 40.0, 0.0], [0.0, 40.0, 0.0]),
            edge(7, [0.0, 40.0, 0.0], [0.0, 0.0, 0.0]),
            edge(8, [0.0, 0.0, 0.0], [40.0, 0.0, 0.0]),
            edge(9, [40.0, 40.0, 0.0], [40.0, 40.0, 20.0]),
            edge(10, [0.0, 0.0, 0.0], [0.0, 0.0, 20.0]),
            edge(11, [40.0, 0.0, 0.0], [40.0, 0.0, 20.0]),
            edge(12, [0.0, 40.0, 0.0], [0.0, 40.0, 20.0]),
        ],
    }
}

/// Cylinder r=10 h=30 on +Z: face 20 top cap (plane +Z), 21 bottom cap
/// (plane -Z), 22 lateral (cylinder surface, axis Z, no single normal).
/// Edges 20 top circle (closed), 21 bottom circle, 22 side seam.
fn cylinder_view() -> TopologyView {
    TopologyView {
        faces: vec![
            face(
                20,
                [0.0, 0.0, 30.0],
                Some([0.0, 0.0, 1.0]),
                Some([0.0, 0.0, 1.0]),
                FaceSurface::Plane,
                vec![20],
            ),
            face(
                21,
                [0.0, 0.0, 0.0],
                Some([0.0, 0.0, -1.0]),
                Some([0.0, 0.0, -1.0]),
                FaceSurface::Plane,
                vec![21],
            ),
            face(
                22,
                [0.0, 0.0, 15.0],
                None,
                Some([0.0, 0.0, 1.0]),
                FaceSurface::Cylinder,
                vec![20, 21, 22],
            ),
        ],
        edges: vec![
            edge(20, [10.0, 0.0, 30.0], [10.0, 0.0, 30.0]),
            edge(21, [10.0, 0.0, 0.0], [10.0, 0.0, 0.0]),
            edge(22, [10.0, 0.0, 0.0], [10.0, 0.0, 30.0]),
        ],
    }
}

fn ids(set: std::collections::BTreeSet<u64>) -> Vec<u64> {
    set.into_iter().collect()
}

#[test]
fn selector_serde_wire_format() {
    let s: Selector = serde_json::from_value(json!({"op": "top_face"})).unwrap();
    assert_eq!(s, Selector::TopFace);
    let s: Selector =
        serde_json::from_value(json!({"op": "faces_normal_to", "dir": [0, 0, 1]})).unwrap();
    assert_eq!(
        s,
        Selector::FacesNormalTo {
            dir: [0.0, 0.0, 1.0]
        }
    );
    let s: Selector =
        serde_json::from_value(json!({"op": "edges_adjacent_to", "faces": {"op": "top_face"}}))
            .unwrap();
    assert_eq!(
        s,
        Selector::EdgesAdjacentTo {
            faces: Box::new(Selector::TopFace)
        }
    );
    // round-trips
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["op"], "edges_adjacent_to");
    // unknown op is a parse error, not a silent default
    assert!(serde_json::from_value::<Selector>(json!({"op": "nope"})).is_err());
}

#[test]
fn target_kinds_are_enforced() {
    assert_eq!(Selector::TopFace.target().unwrap(), SelectorTarget::Face);
    assert_eq!(Selector::AllEdges.target().unwrap(), SelectorTarget::Edge);
    // edges_adjacent_to requires a face operand
    let bad = Selector::EdgesAdjacentTo {
        faces: Box::new(Selector::AllEdges),
    };
    assert!(matches!(
        bad.target(),
        Err(CadError::SelectorKind {
            expected: "faces",
            actual: "edges"
        })
    ));
    // mixed set operands are malformed
    let bad = Selector::Union {
        of: vec![Selector::TopFace, Selector::AllEdges],
    };
    assert!(matches!(bad.target(), Err(CadError::BadSelector(_))));
    // empty operand list
    let bad = Selector::Union { of: vec![] };
    assert!(matches!(bad.target(), Err(CadError::BadSelector(_))));
}

#[test]
fn top_and_bottom_face_on_box() {
    let v = box_view();
    assert_eq!(ids(resolve_faces(&Selector::TopFace, &v).unwrap()), [5]);
    assert_eq!(ids(resolve_faces(&Selector::BottomFace, &v).unwrap()), [6]);
    // generic extreme agrees on a box
    assert_eq!(
        ids(resolve_faces(
            &Selector::FaceExtreme {
                dir: [0.0, 0.0, 1.0]
            },
            &v
        )
        .unwrap()),
        [5]
    );
}

#[test]
fn faces_normal_to_is_signed_and_planar_only() {
    let v = box_view();
    assert_eq!(
        ids(resolve_faces(
            &Selector::FacesNormalTo {
                dir: [0.0, 0.0, 1.0]
            },
            &v
        )
        .unwrap()),
        [5]
    );
    assert_eq!(
        ids(resolve_faces(
            &Selector::FacesNormalTo {
                dir: [0.0, 0.0, -1.0]
            },
            &v
        )
        .unwrap()),
        [6]
    );
    // a cylinder's lateral face has no single normal → only the cap
    let c = cylinder_view();
    assert_eq!(
        ids(resolve_faces(
            &Selector::FacesNormalTo {
                dir: [0.0, 0.0, 1.0]
            },
            &c
        )
        .unwrap()),
        [20]
    );
}

#[test]
fn faces_axis_to_matches_curved_faces_unsigned() {
    let c = cylinder_view();
    // +Z axis: top cap, bottom cap (unsigned) AND lateral surface
    assert_eq!(
        ids(resolve_faces(
            &Selector::FacesAxisTo {
                dir: [0.0, 0.0, 1.0]
            },
            &c
        )
        .unwrap()),
        [20, 21, 22]
    );
    // nothing has a horizontal axis
    assert!(
        resolve_faces(
            &Selector::FacesAxisTo {
                dir: [1.0, 0.0, 0.0]
            },
            &c
        )
        .unwrap()
        .is_empty()
    );
    // faces_of_kind selects the lateral surface alone
    assert_eq!(
        ids(resolve_faces(
            &Selector::FacesOfKind {
                kind: FaceSurface::Cylinder
            },
            &c
        )
        .unwrap()),
        [22]
    );
}

#[test]
fn extreme_is_ambiguous_on_ties() {
    // square plan view: +X and +Y faces tie along the diagonal
    let v = box_view();
    let err = resolve_faces(
        &Selector::FaceExtreme {
            dir: [1.0, 1.0, 0.0],
        },
        &v,
    )
    .unwrap_err();
    match err {
        CadError::SelectorAmbiguous { ids, .. } => assert_eq!(ids, vec![1, 3]),
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

#[test]
fn edges_adjacent_to_collects_boundary_edges() {
    let v = box_view();
    // edges_adjacent_to(top_face) → the four top edges
    let sel = Selector::EdgesAdjacentTo {
        faces: Box::new(Selector::TopFace),
    };
    assert_eq!(ids(resolve_edges(&sel, &v).unwrap()), [1, 2, 3, 4]);
    // by raw face id — the documented `edges_adjacent_to(f)` form
    let sel = Selector::EdgesAdjacentTo {
        faces: Box::new(Selector::FaceIds { ids: vec![5] }),
    };
    assert_eq!(ids(resolve_edges(&sel, &v).unwrap()), [1, 2, 3, 4]);
    // a face that matches nothing contributes no edges
    let sel = Selector::EdgesAdjacentTo {
        faces: Box::new(Selector::FacesOfKind {
            kind: FaceSurface::Sphere,
        }),
    };
    assert!(resolve_edges(&sel, &v).unwrap().is_empty());
}

#[test]
fn edges_extreme_selects_a_set() {
    let v = box_view();
    assert_eq!(
        ids(resolve_edges(
            &Selector::EdgesExtreme {
                dir: [0.0, 0.0, 1.0]
            },
            &v
        )
        .unwrap()),
        [1, 2, 3, 4]
    );
    // cylinder: top circle edge only (the seam's midpoint is mid-height)
    let c = cylinder_view();
    assert_eq!(
        ids(resolve_edges(
            &Selector::EdgesExtreme {
                dir: [0.0, 0.0, 1.0]
            },
            &c
        )
        .unwrap()),
        [20]
    );
}

#[test]
fn set_composition() {
    let v = box_view();
    let union = Selector::Union {
        of: vec![Selector::TopFace, Selector::BottomFace],
    };
    assert_eq!(ids(resolve(&union, &v).unwrap()), [5, 6]);
    let inter = Selector::Intersect {
        of: vec![
            Selector::FacesNormalTo {
                dir: [0.0, 0.0, 1.0],
            },
            Selector::FacesOfKind {
                kind: FaceSurface::Plane,
            },
        ],
    };
    assert_eq!(ids(resolve(&inter, &v).unwrap()), [5]);
    let diff = Selector::Difference {
        base: Box::new(Selector::AllFaces),
        minus: Box::new(Selector::Union {
            of: vec![Selector::TopFace, Selector::BottomFace],
        }),
    };
    assert_eq!(ids(resolve(&diff, &v).unwrap()), [1, 2, 3, 4]);
}

#[test]
fn kind_mismatch_and_stale_and_empty() {
    let v = box_view();
    // face selector where edges are required
    assert!(matches!(
        resolve_edges(&Selector::TopFace, &v),
        Err(CadError::SelectorKind {
            expected: "edges",
            actual: "faces"
        })
    ));
    // edge selector where faces are required
    assert!(matches!(
        resolve_faces(&Selector::AllEdges, &v),
        Err(CadError::SelectorKind {
            expected: "faces",
            actual: "edges"
        })
    ));
    // raw ids that are not on the shape → stale reference
    let err = resolve_edges(&Selector::EdgeIds { ids: vec![1, 999] }, &v).unwrap_err();
    match err {
        CadError::SelectorStale { ids, target } => {
            assert_eq!(ids, vec![999]);
            assert_eq!(target, "edges");
        }
        other => panic!("expected stale, got {other:?}"),
    }
    // empty match is a data result, not an error — callers escalate
    assert!(
        resolve_faces(
            &Selector::FacesOfKind {
                kind: FaceSurface::Torus
            },
            &v
        )
        .unwrap()
        .is_empty()
    );
    // zero direction is malformed
    assert!(matches!(
        resolve_faces(
            &Selector::FacesNormalTo {
                dir: [0.0, 0.0, 0.0]
            },
            &v
        ),
        Err(CadError::BadSelector(_))
    ));
}
