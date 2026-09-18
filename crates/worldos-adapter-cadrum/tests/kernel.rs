//! Real OCCT verification of `CadrumKernel` through the `CadKernel`
//! trait — primitives, booleans, features, transforms, measures,
//! topology, and BRep/STEP/STL round-trips.

use worldos_adapter_cadrum::CadrumKernel;
use worldos_cad::{
    BoolOp, CadError, CadKernel, ShapeId, TessParams, TransformOp, approx_mm, approx_relative,
};

fn kernel() -> CadrumKernel {
    CadrumKernel::new()
}

#[test]
fn box_measures_are_kernel_verified() {
    let k = kernel();
    let b = k.make_box(50.0, 40.0, 20.0).unwrap();
    let m = k.measure(b).unwrap();
    assert!(approx_mm(m.volume_mm3, 50.0 * 40.0 * 20.0));
    assert!(approx_mm(
        m.area_mm2,
        2.0 * (50.0 * 40.0 + 50.0 * 20.0 + 40.0 * 20.0)
    ));
    // OCCT reports bboxes with Precision::Confusion (1e-7) slack.
    for i in 0..3 {
        assert!(approx_mm(m.bbox.min_mm[i], 0.0));
        assert!(approx_mm(m.bbox.max_mm[i], [50.0, 40.0, 20.0][i]));
    }
    let t = k.topology(b).unwrap();
    assert_eq!(t.faces, 6);
    assert_eq!(t.edges, 12);
    assert!(t.is_solid && t.is_valid);
    assert_eq!(t.edge_ids.len(), 12);
    assert_eq!(t.face_ids.len(), 6);
}

#[test]
fn cylinder_and_sphere() {
    let k = kernel();
    let c = k.make_cylinder(10.0, 30.0).unwrap();
    let m = k.measure(c).unwrap();
    // π r² h
    assert!(approx_relative(
        m.volume_mm3,
        std::f64::consts::PI * 100.0 * 30.0
    ));
    let s = k.make_sphere(10.0).unwrap();
    let m = k.measure(s).unwrap();
    assert!(approx_relative(
        m.volume_mm3,
        4.0 / 3.0 * std::f64::consts::PI * 1000.0
    ));
}

#[test]
fn boolean_union_subtract_intersect() {
    let k = kernel();
    let a = k.make_box(40.0, 40.0, 40.0).unwrap();
    let b = k.make_cylinder(10.0, 50.0).unwrap();

    let u = k.boolean(a, b, BoolOp::Union).unwrap();
    let um = k.measure(u).unwrap();
    let va = 40.0_f64.powi(3);
    let vb = std::f64::consts::PI * 100.0 * 50.0;
    // Cylinder axis runs through the box's corner at origin: only a
    // quarter of its cross-section lies inside the box, over height 40.
    let inter = std::f64::consts::PI * 100.0 * 40.0 / 4.0;
    assert!(approx_relative(um.volume_mm3, va + vb - inter));

    let s = k.boolean(a, b, BoolOp::Subtract).unwrap();
    let sm = k.measure(s).unwrap();
    assert!(approx_relative(sm.volume_mm3, va - inter));

    let i = k.boolean(a, b, BoolOp::Intersect).unwrap();
    let im = k.measure(i).unwrap();
    assert!(approx_relative(im.volume_mm3, inter));
}

#[test]
fn fillet_and_chamfer_on_all_edges() {
    let k = kernel();
    let b = k.make_box(40.0, 40.0, 40.0).unwrap();
    let f = k.fillet(b, 3.0, &[]).unwrap(); // empty selector = all edges
    let fm = k.measure(f).unwrap();
    assert!(fm.volume_mm3 < 64000.0 && fm.volume_mm3 > 60000.0);

    let b2 = k.make_box(40.0, 40.0, 40.0).unwrap();
    let c = k.chamfer(b2, 2.0, &[]).unwrap();
    let cm = k.measure(c).unwrap();
    assert!(cm.volume_mm3 < 64000.0 && cm.volume_mm3 > 60000.0);

    // explicit edge selector: first two edges only
    let b3 = k.make_box(40.0, 40.0, 40.0).unwrap();
    let t = k.topology(b3).unwrap();
    let f2 = k.fillet(b3, 2.0, &t.edge_ids[..2]).unwrap();
    let fm2 = k.measure(f2).unwrap();
    assert!(fm2.volume_mm3 < 64000.0 && fm2.volume_mm3 > 63800.0);
}

#[test]
fn transform_translate_rotate_scale() {
    let k = kernel();
    let b = k.make_box(10.0, 10.0, 10.0).unwrap();
    let t = k
        .transform(
            b,
            &[
                TransformOp::Translate {
                    delta_mm: [100.0, 0.0, 0.0],
                },
                TransformOp::RotateAxis {
                    origin_mm: [100.0, 5.0, 5.0],
                    dir: [0.0, 0.0, 1.0],
                    angle_rad: std::f64::consts::FRAC_PI_2,
                },
                TransformOp::Scale {
                    center_mm: [100.0, 5.0, 5.0],
                    factor: 2.0,
                },
            ],
        )
        .unwrap();
    let m = k.measure(t).unwrap();
    assert!(approx_relative(m.volume_mm3, 8000.0)); // 10³ × 2³
    // rotated 90° about Z then scaled ×2 about the box center
    assert!(approx_mm(m.bbox.min_mm[0], 90.0));
    assert!(approx_mm(m.bbox.max_mm[0], 110.0));
}

#[test]
fn brep_and_step_round_trip_preserve_measures() {
    let k = kernel();
    let a = k.make_box(40.0, 40.0, 40.0).unwrap();
    let c = k.make_cylinder(10.0, 50.0).unwrap();
    let s = k.boolean(a, c, BoolOp::Subtract).unwrap();
    let before = k.measure(s).unwrap();

    // BRep round-trip
    let brep = k.export_brep(s).unwrap();
    assert!(!brep.is_empty());
    let s2 = k.import_brep(&brep).unwrap();
    let m2 = k.measure(s2).unwrap();
    assert!(approx_relative(m2.volume_mm3, before.volume_mm3));
    assert!(approx_relative(m2.area_mm2, before.area_mm2));

    // STEP round-trip
    let step = k.export_step(s).unwrap();
    let text = String::from_utf8_lossy(&step);
    assert!(text.contains("ISO-10303-21"));
    let s3 = k.import_step(&step).unwrap();
    let m3 = k.measure(s3).unwrap();
    assert!(approx_relative(m3.volume_mm3, before.volume_mm3));
}

#[test]
fn stl_export_and_mesh() {
    let k = kernel();
    let b = k.make_box(10.0, 10.0, 10.0).unwrap();
    let stl = k.export_stl(b, TessParams::default()).unwrap();
    let text = String::from_utf8_lossy(&stl);
    assert!(text.contains("solid") || stl.len() > 84); // ascii or binary stl

    let mesh = k.mesh(b, TessParams::default()).unwrap();
    assert_eq!(mesh.indices.len() % 3, 0);
    assert!(!mesh.positions.is_empty());
    assert_eq!(mesh.face_ids.len(), mesh.indices.len() / 3);
}

#[test]
fn handle_lifecycle_and_errors() {
    let k = kernel();
    let b = k.make_box(1.0, 1.0, 1.0).unwrap();
    let c = k.clone_shape(b).unwrap();
    assert_ne!(b, c);
    k.drop_shape(b);
    assert!(matches!(k.measure(b), Err(CadError::UnknownShape(_))));
    assert!(k.measure(c).is_ok());
    k.drop_shape(ShapeId(9999)); // unknown id: idempotent, no panic

    assert!(k.make_box(0.0, 1.0, 1.0).is_err());
    assert!(k.make_cylinder(-1.0, 1.0).is_err());
    assert!(k.import_step(b"not a step file").is_err());
}
