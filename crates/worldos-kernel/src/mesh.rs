//! Deterministic tessellation of the analytic `geom:*` primitives.
//!
//! Sibling of [`crate::measure`]: primitives carry analytic shape data
//! (`geom:geometry.size`, `core:transform.scale`/`position`), so a mesh
//! can be generated without a B-rep kernel. The output feeds the
//! `geometry.export` command — binary STL / OBJ writers live here too.
//!
//! ## Conventions (match `kernel::measure`)
//!
//! - dims = `size` × `scale` (see [`measure::object_dims`]); the mesh is
//!   generated centered at origin, then translated by
//!   [`measure::object_position`]. Rotation is NOT applied — the same
//!   limitation `object_bbox` documents applies here.
//! - `cube`: axis-aligned box `[dx, dy, dz]`.
//! - `sphere`: ellipsoid with semi-axes `d/2` (a uniformly scaled sphere
//!   stays a sphere; non-uniform scale yields the honest ellipsoid).
//! - `cylinder`/`cone`: axis `+z`, height `d[2]`, elliptical base
//!   `rx = d[0]/2`, `ry = d[1]/2`, centered on `z = 0` (cone apex up).
//! - `torus`: ring lies flat in the `xz` ground plane (axis `+y`),
//!   `d[0]`/`d[2]` are the OUTER ring diameters, `d[1]` the tube
//!   diameter — this keeps the mesh inside the bbox `object_bbox`
//!   claims. The centerline radii are `(d[0]-d[1])/2`, `(d[2]-d[1])/2`.
//! - `plane`: single-sided quad spanning `d[0] × d[1]` in the `xy`
//!   plane at `z = 0` (matches `measure_primitive`'s `d[0]*d[1]` area).
//!   A plane is a surface, not a solid — its STL is an open mesh.
//!
//! B-rep `cad:body` objects are out of scope here — the optional OCCT
//! adapter tessellates those (`cad.export_stl`). Unknown `geom:*` kinds
//! fail closed instead of guessing a shape.

use crate::error::KernelError;
use crate::model::Object;
use std::f64::consts::PI;

/// Triangles around a circle of revolution (cylinder/cone bases, torus
/// ring, sphere longitude). Fixed so identical inputs produce identical
/// bytes — determinism is part of the export contract.
pub const RADIAL_SEGMENTS: u32 = 32;
/// Sphere latitude bands between the two poles.
pub const SPHERE_STACKS: u32 = 16;
/// Torus tube circumference subdivisions (32 keeps the inscribed
/// polygon within ~0.7% of the ideal circular tube area).
pub const TORUS_TUBE_SEGMENTS: u32 = 32;

/// An indexed triangle mesh: `indices` holds triangle corners into
/// `positions` (three per face, counter-clockwise when seen from
/// outside the solid).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Number of triangles (`indices.len() / 3`).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Translate every vertex by `d` (used to place meshes at
    /// `core:transform.position`).
    pub fn translate(&mut self, d: [f64; 3]) {
        for p in &mut self.positions {
            p[0] += d[0];
            p[1] += d[1];
            p[2] += d[2];
        }
    }

    /// Axis-aligned bounding box `[min, max]`; `None` for an empty mesh.
    pub fn bbox(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut it = self.positions.iter();
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        Some((lo, hi))
    }
}

fn bad(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// Tessellate a primitive `kind` at `dims` (full x/y/z extents),
/// centered at the origin. Errors on unknown kinds and non-positive or
/// non-finite dimensions — exporting a guessed shape is worse than
/// refusing.
pub fn tessellate(kind: &str, dims: [f64; 3]) -> Result<Mesh, KernelError> {
    if !dims.iter().all(|d| d.is_finite()) {
        return Err(bad(format!("non-finite dimensions {dims:?}")));
    }
    match kind {
        "cube" | "box" => {
            positive(kind, dims)?;
            Ok(cube(dims))
        }
        "sphere" => {
            positive(kind, dims)?;
            Ok(sphere(dims))
        }
        "cylinder" => {
            positive(kind, dims)?;
            Ok(cylinder(dims))
        }
        "cone" => {
            positive(kind, dims)?;
            Ok(cone(dims))
        }
        "torus" => torus(dims),
        "plane" => {
            if dims[0] <= 0.0 || dims[1] <= 0.0 {
                return Err(bad(format!("plane needs positive x/y dims, got {dims:?}")));
            }
            Ok(plane(dims))
        }
        other => Err(bad(format!("no tessellation for primitive kind `{other}`"))),
    }
}

fn positive(kind: &str, dims: [f64; 3]) -> Result<(), KernelError> {
    if dims.iter().all(|d| *d > 0.0) {
        Ok(())
    } else {
        Err(bad(format!("{kind} needs positive dims, got {dims:?}")))
    }
}

/// Mesh for an object's analytic geometry, placed at its
/// `core:transform.position` (world-space, like `object_bbox`).
/// Errors when the object carries no `geom:geometry` component.
pub fn object_mesh(obj: &Object) -> Result<Mesh, KernelError> {
    let kind = crate::measure::object_kind(obj)
        .ok_or_else(|| bad(format!("object `{}` has no geom:geometry kind", obj.name)))?
        .to_string();
    let dims = crate::measure::object_dims(obj)
        .ok_or_else(|| bad(format!("object `{}` has no geometry dims", obj.name)))?;
    let mut mesh = tessellate(&kind, dims)?;
    mesh.translate(crate::measure::object_position(obj));
    Ok(mesh)
}

// ------------------------------------------------------------------ solids

fn cube(d: [f64; 3]) -> Mesh {
    let (x, y, z) = (d[0] / 2.0, d[1] / 2.0, d[2] / 2.0);
    let positions = vec![
        [-x, -y, -z], // 0
        [x, -y, -z],  // 1
        [x, y, -z],   // 2
        [-x, y, -z],  // 3
        [-x, -y, z],  // 4
        [x, -y, z],   // 5
        [x, y, z],    // 6
        [-x, y, z],   // 7
    ];
    // outward-facing quads as two triangles each
    let indices = vec![
        0, 3, 2, 0, 2, 1, // -z
        4, 5, 6, 4, 6, 7, // +z
        0, 1, 5, 0, 5, 4, // -y
        3, 7, 6, 3, 6, 2, // +y
        0, 4, 7, 0, 7, 3, // -x
        1, 2, 6, 1, 6, 5, // +x
    ];
    Mesh { positions, indices }
}

fn sphere(d: [f64; 3]) -> Mesh {
    let (rx, ry, rz) = (d[0] / 2.0, d[1] / 2.0, d[2] / 2.0);
    let (lon, stacks) = (RADIAL_SEGMENTS, SPHERE_STACKS);
    let mut positions = vec![[0.0, 0.0, -rz]]; // south pole
    for i in 1..stacks {
        let phi = -PI / 2.0 + PI * (i as f64) / (stacks as f64);
        let (c, s) = (phi.cos(), phi.sin());
        for k in 0..lon {
            let theta = 2.0 * PI * (k as f64) / (lon as f64);
            positions.push([rx * c * theta.cos(), ry * c * theta.sin(), rz * s]);
        }
    }
    let north = positions.len() as u32;
    positions.push([0.0, 0.0, rz]);

    let ring = |i: u32, k: u32| 1 + (i - 1) * lon + (k % lon);
    let mut indices = Vec::new();
    // south cap: (pole, next, cur) winds outward (normal -z)
    for k in 0..lon {
        indices.extend_from_slice(&[0, ring(1, k + 1), ring(1, k)]);
    }
    // interior bands
    for i in 1..stacks - 1 {
        for k in 0..lon {
            let (a, b) = (ring(i, k), ring(i, k + 1));
            let (c, d2) = (ring(i + 1, k), ring(i + 1, k + 1));
            indices.extend_from_slice(&[a, b, d2, a, d2, c]);
        }
    }
    // north cap
    let top = stacks - 1;
    for k in 0..lon {
        indices.extend_from_slice(&[ring(top, k), ring(top, k + 1), north]);
    }
    Mesh { positions, indices }
}

/// Ring of `n` ellipse vertices at height `z`.
fn ring_vertices(positions: &mut Vec<[f64; 3]>, rx: f64, ry: f64, z: f64) {
    for k in 0..RADIAL_SEGMENTS {
        let theta = 2.0 * PI * (k as f64) / (RADIAL_SEGMENTS as f64);
        positions.push([rx * theta.cos(), ry * theta.sin(), z]);
    }
}

fn cylinder(d: [f64; 3]) -> Mesh {
    let (rx, ry, h) = (d[0] / 2.0, d[1] / 2.0, d[2] / 2.0);
    let n = RADIAL_SEGMENTS;
    let mut positions = Vec::new();
    ring_vertices(&mut positions, rx, ry, -h); // bottom ring 0..n
    ring_vertices(&mut positions, rx, ry, h); // top ring    n..2n
    let (cb, ct) = (2 * n, 2 * n + 1);
    positions.push([0.0, 0.0, -h]);
    positions.push([0.0, 0.0, h]);
    let mut indices = Vec::new();
    for k in 0..n {
        let (b0, b1) = (k, (k + 1) % n);
        let (t0, t1) = (n + k, n + (k + 1) % n);
        // side (outward), bottom cap (-z), top cap (+z)
        indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
        indices.extend_from_slice(&[cb, b1, b0]);
        indices.extend_from_slice(&[ct, t0, t1]);
    }
    Mesh { positions, indices }
}

fn cone(d: [f64; 3]) -> Mesh {
    let (rx, ry, h) = (d[0] / 2.0, d[1] / 2.0, d[2] / 2.0);
    let n = RADIAL_SEGMENTS;
    let mut positions = Vec::new();
    ring_vertices(&mut positions, rx, ry, -h); // base ring 0..n
    let (apex, cb) = (n, n + 1);
    positions.push([0.0, 0.0, h]);
    positions.push([0.0, 0.0, -h]);
    let mut indices = Vec::new();
    for k in 0..n {
        let (b0, b1) = (k, (k + 1) % n);
        indices.extend_from_slice(&[b0, b1, apex]); // flank, outward
        indices.extend_from_slice(&[cb, b1, b0]); // base cap, -z
    }
    Mesh { positions, indices }
}

fn torus(d: [f64; 3]) -> Result<Mesh, KernelError> {
    let tube = d[1] / 2.0;
    let (cx, cz) = (d[0] / 2.0 - tube, d[2] / 2.0 - tube);
    if tube <= 0.0 || cx <= 0.0 || cz <= 0.0 {
        return Err(bad(format!(
            "torus needs 0 < tube_d < min(ring_dx, ring_dz), got {d:?}"
        )));
    }
    let (ns, nt) = (RADIAL_SEGMENTS, TORUS_TUBE_SEGMENTS);
    let mut positions = Vec::new();
    for s in 0..ns {
        let a = 2.0 * PI * (s as f64) / (ns as f64);
        let (sa, ca) = (a.sin(), a.cos());
        for t in 0..nt {
            let b = 2.0 * PI * (t as f64) / (nt as f64);
            let (sb, cb) = (b.sin(), b.cos());
            positions.push([(cx + tube * cb) * ca, tube * sb, (cz + tube * cb) * sa]);
        }
    }
    let v = |s: u32, t: u32| (s % ns) * nt + (t % nt);
    let mut indices = Vec::new();
    for s in 0..ns {
        for t in 0..nt {
            let (a, b) = (v(s, t), v(s + 1, t));
            let (c, d2) = (v(s + 1, t + 1), v(s, t + 1));
            indices.extend_from_slice(&[a, c, b, a, d2, c]);
        }
    }
    Ok(Mesh { positions, indices })
}

fn plane(d: [f64; 3]) -> Mesh {
    let (x, y) = (d[0] / 2.0, d[1] / 2.0);
    Mesh {
        positions: vec![[-x, -y, 0.0], [x, -y, 0.0], [x, y, 0.0], [-x, y, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3], // +z
    }
}

// ------------------------------------------------------------------ writers

/// Facet normal of triangle `abc`, normalized; `[0,0,0]` when the
/// triangle is degenerate (STL readers accept a zero normal and
/// recompute from the winding).
fn facet_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        [n[0] / len, n[1] / len, n[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Binary STL (80-byte header + u32 count + 50 bytes per facet).
/// Deterministic: fixed header text, little-endian f32 fields, no
/// timestamps — identical meshes produce identical files.
pub fn to_binary_stl(mesh: &Mesh) -> Vec<u8> {
    const HEADER: &[u8] = b"worldos geometry.export binary stl";
    let mut out = Vec::with_capacity(84 + mesh.triangle_count() * 50);
    out.extend_from_slice(HEADER);
    out.resize(80, 0);
    out.extend_from_slice(&(mesh.triangle_count() as u32).to_le_bytes());
    let push_f32 = |out: &mut Vec<u8>, v: f64| out.extend_from_slice(&(v as f32).to_le_bytes());
    for &[i0, i1, i2] in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = [
            mesh.positions[i0 as usize],
            mesh.positions[i1 as usize],
            mesh.positions[i2 as usize],
        ];
        for v in facet_normal(a, b, c) {
            push_f32(&mut out, v);
        }
        for p in [a, b, c] {
            for v in p {
                push_f32(&mut out, v);
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// Wavefront OBJ: `v` lines plus 1-based `f` triangles (no normals —
/// positions only; importers recompute). Deterministic text.
pub fn to_obj(mesh: &Mesh) -> Vec<u8> {
    let mut s = String::from("# worldos geometry.export\n");
    for p in &mesh.positions {
        s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
        s.push_str(&format!("f {} {} {}\n", a + 1, b + 1, c + 1));
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed volume via divergence theorem — positive iff the mesh is
    /// closed with outward-wound triangles.
    fn signed_volume(m: &Mesh) -> f64 {
        let mut v = 0.0;
        for &[i0, i1, i2] in m.indices.as_chunks::<3>().0 {
            let [a, b, c] = [
                m.positions[i0 as usize],
                m.positions[i1 as usize],
                m.positions[i2 as usize],
            ];
            v += a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
        v / 6.0
    }

    #[test]
    fn primitives_have_positive_outward_volume() {
        for (kind, d) in [
            ("cube", [2.0, 3.0, 4.0]),
            ("sphere", [2.0, 2.0, 2.0]),
            ("cylinder", [2.0, 2.0, 5.0]),
            ("cone", [2.0, 2.0, 3.0]),
            ("torus", [4.0, 1.0, 4.0]),
        ] {
            let m = tessellate(kind, d).unwrap();
            assert!(
                signed_volume(&m) > 0.0,
                "{kind} wound inward (signed volume {})",
                signed_volume(&m)
            );
        }
    }

    #[test]
    fn volumes_approach_analytic_measures() {
        let sphere = tessellate("sphere", [2.0, 2.0, 2.0]).unwrap();
        let exact = 4.0 / 3.0 * PI; // r = 1
        assert!((signed_volume(&sphere) / exact - 1.0).abs() < 0.02);
        let cyl = tessellate("cylinder", [2.0, 2.0, 5.0]).unwrap();
        assert!((signed_volume(&cyl) / (PI * 5.0) - 1.0).abs() < 0.02);
        let torus = tessellate("torus", [4.0, 1.0, 4.0]).unwrap();
        // outer radius 2, tube 0.5 → centerline R = 1.5
        let exact_t = 2.0 * PI * PI * 1.5 * 0.25;
        assert!((signed_volume(&torus) / exact_t - 1.0).abs() < 0.02);
    }

    #[test]
    fn mesh_stays_inside_claimed_bbox() {
        for (kind, d) in [
            ("cube", [2.0, 3.0, 4.0]),
            ("sphere", [2.0, 4.0, 6.0]),
            ("cylinder", [2.0, 3.0, 5.0]),
            ("cone", [2.0, 3.0, 5.0]),
            ("torus", [4.0, 1.0, 5.0]),
            ("plane", [3.0, 2.0, 1.0]),
        ] {
            let m = tessellate(kind, d).unwrap();
            let (lo, hi) = m.bbox().unwrap();
            for i in 0..3 {
                assert!(lo[i] >= -d[i] / 2.0 - 1e-9, "{kind} lo[{i}]={}", lo[i]);
                assert!(hi[i] <= d[i] / 2.0 + 1e-9, "{kind} hi[{i}]={}", hi[i]);
            }
        }
    }

    #[test]
    fn bad_inputs_fail_closed() {
        assert!(tessellate("klein-bottle", [1.0; 3]).is_err());
        assert!(tessellate("cube", [0.0, 1.0, 1.0]).is_err());
        assert!(tessellate("sphere", [f64::NAN; 3]).is_err());
        // tube thicker than the ring is a self-intersecting spindle
        assert!(tessellate("torus", [2.0, 4.0, 2.0]).is_err());
    }

    #[test]
    fn binary_stl_layout_and_determinism() {
        let m = tessellate("cube", [2.0, 2.0, 2.0]).unwrap();
        let stl = to_binary_stl(&m);
        assert_eq!(stl.len(), 84 + 12 * 50);
        assert!(stl.starts_with(b"worldos geometry.export"));
        let count = u32::from_le_bytes(stl[80..84].try_into().unwrap());
        assert_eq!(count, 12);
        // first facet of a cube is the -z face: normal (0,0,-1)
        let nz = f32::from_le_bytes(stl[92..96].try_into().unwrap());
        assert_eq!(nz, -1.0);
        assert_eq!(stl, to_binary_stl(&m), "same mesh must give same bytes");
    }

    #[test]
    fn obj_is_positions_and_faces() {
        let m = tessellate("cube", [1.0, 1.0, 1.0]).unwrap();
        let text = String::from_utf8(to_obj(&m)).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("v ")).count(), 8);
        assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 12);
        assert!(text.contains("f 1 4 3")); // cube face 0 is verts 0,3,2
    }

    #[test]
    fn object_mesh_applies_position_not_rotation() {
        let mut obj = Object::new("geom:cube", "c", &crate::ids::ActorId::new("t"));
        obj.set_component(crate::model::Component::new(
            crate::known::components::GEOMETRY,
            serde_json::json!({"kind": "cube", "size": 2.0}),
        ));
        obj.set_component(crate::model::Component::new(
            crate::known::components::TRANSFORM,
            serde_json::json!({"position": [10.0, 0.0, 0.0], "scale": [1, 1, 1]}),
        ));
        let m = object_mesh(&obj).unwrap();
        let (lo, hi) = m.bbox().unwrap();
        assert_eq!(lo[0], 9.0);
        assert_eq!(hi[0], 11.0);
        // scale multiplies size
        obj.set_component(crate::model::Component::new(
            crate::known::components::TRANSFORM,
            serde_json::json!({"position": [0, 0, 0], "scale": [2, 2, 2]}),
        ));
        let m = object_mesh(&obj).unwrap();
        assert_eq!(m.bbox().unwrap().1[0], 2.0);
    }
}
