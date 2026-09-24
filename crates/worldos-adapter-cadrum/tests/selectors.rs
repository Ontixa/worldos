//! Selector resolution against the real OCCT kernel: `topology_view`
//! must report truthful normals/axes/centers, and selectors must
//! resolve the same way before and after a BRep round-trip (kernel
//! ids may or may not survive serialization — geometric resolution is
//! what makes that irrelevant).

use worldos_adapter_cadrum::CadrumKernel;
use worldos_cad::{
    CadError, CadKernel, FaceSurface, Selector, approx_mm, resolve_edges, resolve_faces,
};

fn kernel() -> CadrumKernel {
    CadrumKernel::new()
}

fn face_normal(view: &worldos_cad::TopologyView, id: u64) -> [f64; 3] {
    view.faces
        .iter()
        .find(|f| f.id == id)
        .and_then(|f| f.normal)
        .expect("face has a normal")
}

#[test]
fn box_topology_view_reports_planes_with_outward_normals() {
    let k = kernel();
    let b = k.make_box(50.0, 40.0, 20.0).unwrap();
    let view = k.topology_view(b).unwrap();
    assert_eq!(view.faces.len(), 6);
    assert_eq!(view.edges.len(), 12);
    for f in &view.faces {
        assert_eq!(f.surface, FaceSurface::Plane);
        assert!(f.normal.is_some(), "plane face must report a normal");
        assert!(f.axis.is_some(), "plane face must report an axis");
        assert!(f.area_mm2 > 0.0);
    }
    // centers sit on the faces: +Z face at z=20, -Z at z=0
    let top = resolve_faces(&Selector::TopFace, &view).unwrap();
    assert_eq!(top.len(), 1);
    let n = face_normal(&view, *top.iter().next().unwrap());
    assert!(approx_mm(n[2], 1.0), "top face normal must be +Z: {n:?}");
    let center = view
        .faces
        .iter()
        .find(|f| f.id == *top.iter().next().unwrap())
        .unwrap()
        .center_mm;
    assert!(approx_mm(center[2], 20.0));

    let bottom = resolve_faces(&Selector::BottomFace, &view).unwrap();
    let n = face_normal(&view, *bottom.iter().next().unwrap());
    assert!(approx_mm(n[2], -1.0));

    // every face's boundary edges exist in the edge table
    let edge_ids: std::collections::BTreeSet<u64> = view.edges.iter().map(|e| e.id).collect();
    for f in &view.faces {
        assert!(!f.edge_ids.is_empty());
        for e in &f.edge_ids {
            assert!(edge_ids.contains(e));
        }
    }
}

#[test]
fn cylinder_side_face_is_selectable() {
    let k = kernel();
    let c = k.make_cylinder(10.0, 30.0).unwrap();
    let view = k.topology_view(c).unwrap();
    assert_eq!(view.faces.len(), 3);

    // top_face → the +Z cap only
    let top = resolve_faces(&Selector::TopFace, &view).unwrap();
    assert_eq!(top.len(), 1);
    let top_id = *top.iter().next().unwrap();
    assert_eq!(
        view.faces.iter().find(|f| f.id == top_id).unwrap().surface,
        FaceSurface::Plane
    );

    // the lateral surface: cylinder kind, axis parallel to Z
    let side = resolve_faces(
        &Selector::FacesOfKind {
            kind: FaceSurface::Cylinder,
        },
        &view,
    )
    .unwrap();
    assert_eq!(side.len(), 1);
    // its boundary edges are the two cap circles (+ seam if present)
    let ring = resolve_edges(
        &Selector::EdgesAdjacentTo {
            faces: Box::new(Selector::FacesOfKind {
                kind: FaceSurface::Cylinder,
            }),
        },
        &view,
    )
    .unwrap();
    assert!(ring.len() >= 2);

    // axis selector (unsigned): caps + lateral all have a Z-parallel axis
    let z_faces = resolve_faces(
        &Selector::FacesAxisTo {
            dir: [0.0, 0.0, 1.0],
        },
        &view,
    )
    .unwrap();
    assert_eq!(z_faces.len(), 3);
}

#[test]
fn sphere_has_no_top_face() {
    let k = kernel();
    let s = k.make_sphere(10.0).unwrap();
    let view = k.topology_view(s).unwrap();
    assert_eq!(view.faces.len(), 1);
    assert_eq!(view.faces[0].surface, FaceSurface::Sphere);
    // honest empty: a sphere has no planar upward face
    assert!(resolve_faces(&Selector::TopFace, &view).unwrap().is_empty());
    assert!(
        resolve_faces(
            &Selector::FacesNormalTo {
                dir: [0.0, 0.0, 1.0]
            },
            &view
        )
        .unwrap()
        .is_empty()
    );
    // but face_extreme still picks *something* — the whole sphere
    assert_eq!(
        resolve_faces(
            &Selector::FaceExtreme {
                dir: [0.0, 0.0, 1.0]
            },
            &view
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn ambiguous_extreme_and_stale_ids_fail_structurally() {
    let k = kernel();
    // square box: +X and +Y faces tie along the [1,1,0] diagonal
    let b = k.make_box(40.0, 40.0, 20.0).unwrap();
    let view = k.topology_view(b).unwrap();
    let err = resolve_faces(
        &Selector::FaceExtreme {
            dir: [1.0, 1.0, 0.0],
        },
        &view,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::SelectorAmbiguous { .. }), "{err:?}");

    // ids that are not on this shape are a stale reference
    let err = resolve_edges(
        &Selector::EdgeIds {
            ids: vec![u64::MAX],
        },
        &view,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::SelectorStale { .. }), "{err:?}");
}

#[test]
fn selectors_resolve_identically_after_brep_round_trip() {
    let k = kernel();
    let b = k.make_box(50.0, 40.0, 20.0).unwrap();
    let before = resolve_faces(&Selector::TopFace, &k.topology_view(b).unwrap()).unwrap();
    assert_eq!(before.len(), 1);

    let bytes = k.export_brep(b).unwrap();
    let b2 = k.import_brep(&bytes).unwrap();
    let view2 = k.topology_view(b2).unwrap();
    let after = resolve_faces(&Selector::TopFace, &view2).unwrap();
    assert_eq!(after.len(), 1);
    // The face still sits at z=20 with normal +Z — whether or not the
    // kernel reused the same id. Geometric truth, not identity.
    let c = view2
        .faces
        .iter()
        .find(|f| f.id == *after.iter().next().unwrap())
        .unwrap();
    assert!(approx_mm(c.center_mm[2], 20.0));
    assert!(approx_mm(c.normal.unwrap()[2], 1.0));

    // adjacency resolves through the re-imported topology too
    let ring = resolve_edges(
        &Selector::EdgesAdjacentTo {
            faces: Box::new(Selector::TopFace),
        },
        &view2,
    )
    .unwrap();
    assert_eq!(ring.len(), 4);
}
