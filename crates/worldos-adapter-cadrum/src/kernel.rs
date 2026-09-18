//! `CadrumKernel`: [`CadKernel`] over cadrum / OCCT 8.0.1.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use cadrum::{Boolean, DVec3, Solid, Tessellation};
use worldos_cad::error::CadError;
use worldos_cad::kernel::CadKernel;
use worldos_cad::types::{
    BBox, BoolOp, Measures, MeshData, ShapeId, TessParams, Topology, TransformOp,
};

pub struct CadrumKernel {
    shapes: Mutex<HashMap<u64, Solid>>,
    next: AtomicU64,
}

impl Default for CadrumKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl CadrumKernel {
    pub fn new() -> Self {
        Self {
            shapes: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// Lock the shape table, recovering from poisoning — a panicked
    /// writer can at worst leak a handle, so carry on.
    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Solid>> {
        self.shapes.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register(&self, solid: Solid) -> ShapeId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.table().insert(id, solid);
        ShapeId(id)
    }

    /// Run `f` against the solid behind `id`.
    fn with<T>(&self, id: ShapeId, f: impl FnOnce(&Solid) -> T) -> Result<T, CadError> {
        let table = self.table();
        let solid = table.get(&id.0).ok_or(CadError::UnknownShape(id))?;
        Ok(f(solid))
    }

    fn err(e: impl std::fmt::Display) -> CadError {
        CadError::Kernel(e.to_string())
    }

    fn positive(name: &str, v: f64) -> Result<f64, CadError> {
        if v.is_finite() && v > 0.0 {
            Ok(v)
        } else {
            Err(CadError::InvalidInput(format!(
                "{name} must be > 0, got {v}"
            )))
        }
    }

    /// Resolve `ids` (empty = all) to the solid's edge objects.
    fn select_edges<'a>(solid: &'a Solid, ids: &[u64]) -> Vec<&'a cadrum::Edge> {
        if ids.is_empty() {
            solid.iter_edge().collect()
        } else {
            let want: HashSet<u64> = ids.iter().copied().collect();
            solid
                .iter_edge()
                .filter(|e| want.contains(&e.id()))
                .collect()
        }
    }

    fn tess(p: TessParams) -> Tessellation {
        Tessellation {
            deflection_linear: p.deflection_linear,
            deflection_angular: p.deflection_angular,
            relative_linear: p.relative,
        }
    }
}

impl CadKernel for CadrumKernel {
    fn name(&self) -> &'static str {
        "occt-8.0.1-cadrum"
    }

    fn make_box(&self, sx_mm: f64, sy_mm: f64, sz_mm: f64) -> Result<ShapeId, CadError> {
        let (x, y, z) = (
            Self::positive("sx_mm", sx_mm)?,
            Self::positive("sy_mm", sy_mm)?,
            Self::positive("sz_mm", sz_mm)?,
        );
        Ok(self.register(Solid::cube(DVec3::ZERO, DVec3::new(x, y, z))))
    }

    fn make_cylinder(&self, radius_mm: f64, height_mm: f64) -> Result<ShapeId, CadError> {
        let r = Self::positive("radius_mm", radius_mm)?;
        let h = Self::positive("height_mm", height_mm)?;
        Ok(self.register(Solid::cylinder(r, DVec3::new(0.0, 0.0, h))))
    }

    fn make_sphere(&self, radius_mm: f64) -> Result<ShapeId, CadError> {
        let r = Self::positive("radius_mm", radius_mm)?;
        Ok(self.register(Solid::sphere(r)))
    }

    fn boolean(&self, a: ShapeId, b: ShapeId, op: BoolOp) -> Result<ShapeId, CadError> {
        let mut table = self.table();
        let sa = table.get(&a.0).ok_or(CadError::UnknownShape(a))?;
        let sb = table.get(&b.0).ok_or(CadError::UnknownShape(b))?;
        let result = match op {
            BoolOp::Union => (Boolean::from(sa) + sb).build(),
            BoolOp::Subtract => (Boolean::from(sa) - sb).build(),
            BoolOp::Intersect => (Boolean::from(sa) * sb).build(),
        }
        .map_err(Self::err)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        table.insert(id, result);
        Ok(ShapeId(id))
    }

    fn fillet(&self, s: ShapeId, radius_mm: f64, edges: &[u64]) -> Result<ShapeId, CadError> {
        let r = Self::positive("radius_mm", radius_mm)?;
        let mut table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let sel = Self::select_edges(solid, edges);
        if sel.is_empty() {
            return Err(CadError::InvalidInput("no edges matched selector".into()));
        }
        let out = solid.fillet_edges(r, sel).map_err(Self::err)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        table.insert(id, out);
        Ok(ShapeId(id))
    }

    fn chamfer(&self, s: ShapeId, distance_mm: f64, edges: &[u64]) -> Result<ShapeId, CadError> {
        let d = Self::positive("distance_mm", distance_mm)?;
        let mut table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let sel = Self::select_edges(solid, edges);
        if sel.is_empty() {
            return Err(CadError::InvalidInput("no edges matched selector".into()));
        }
        let out = solid.chamfer_edges(d, sel).map_err(Self::err)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        table.insert(id, out);
        Ok(ShapeId(id))
    }

    fn transform(&self, s: ShapeId, ops: &[TransformOp]) -> Result<ShapeId, CadError> {
        let mut table = self.table();
        let mut out = table.get(&s.0).ok_or(CadError::UnknownShape(s))?.clone();
        for op in ops {
            out = match *op {
                TransformOp::Translate { delta_mm } => out.translate(DVec3::from_array(delta_mm)),
                TransformOp::RotateAxis {
                    origin_mm,
                    dir,
                    angle_rad,
                } => out.rotate(
                    DVec3::from_array(origin_mm),
                    DVec3::from_array(dir),
                    angle_rad,
                ),
                TransformOp::Scale { center_mm, factor } => out.scale(
                    DVec3::from_array(center_mm),
                    Self::positive("factor", factor)?,
                ),
            };
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        table.insert(id, out);
        Ok(ShapeId(id))
    }

    fn measure(&self, s: ShapeId) -> Result<Measures, CadError> {
        self.with(s, |solid| {
            let [lo, hi] = solid.bounding_box();
            let c = solid.center();
            Measures {
                volume_mm3: solid.volume(),
                area_mm2: solid.area(),
                bbox: BBox {
                    min_mm: lo.to_array(),
                    max_mm: hi.to_array(),
                },
                center_mm: c.to_array(),
            }
        })
    }

    fn topology(&self, s: ShapeId) -> Result<Topology, CadError> {
        self.with(s, |solid| {
            let edge_ids: Vec<u64> = solid.iter_edge().map(|e| e.id()).collect();
            let face_ids: Vec<u64> = solid.iter_face().map(|f| f.id()).collect();
            // Operational validity: OCCT exposes no BRepCheck here; a
            // well-formed solid has positive volume and real topology.
            let is_valid = solid.volume() > 0.0 && !edge_ids.is_empty() && !face_ids.is_empty();
            Topology {
                solids: 1,
                faces: face_ids.len() as u32,
                edges: edge_ids.len() as u32,
                is_solid: true,
                is_valid,
                edge_ids,
                face_ids,
            }
        })
    }

    fn mesh(&self, s: ShapeId, params: TessParams) -> Result<MeshData, CadError> {
        let table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let mesh = Solid::mesh([solid], Self::tess(params)).map_err(Self::err)?;
        Ok(MeshData {
            positions: mesh.vertices.iter().map(|v| v.to_array()).collect(),
            normals: mesh.normals.iter().map(|v| v.to_array()).collect(),
            indices: mesh.indices.iter().map(|&i| i as u32).collect(),
            face_ids: mesh.face_ids,
        })
    }

    fn export_brep(&self, s: ShapeId) -> Result<Vec<u8>, CadError> {
        let table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let mut out = Vec::new();
        Solid::write_brep([solid], &mut out).map_err(Self::err)?;
        Ok(out)
    }

    fn import_brep(&self, bytes: &[u8]) -> Result<ShapeId, CadError> {
        let mut cur = Cursor::new(bytes);
        let solids = Solid::read_brep(&mut cur).map_err(Self::err)?;
        let first = solids
            .into_iter()
            .next()
            .ok_or_else(|| CadError::Kernel("brep contained no solids".into()))?;
        Ok(self.register(first))
    }

    fn export_step(&self, s: ShapeId) -> Result<Vec<u8>, CadError> {
        let table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let mut out = Vec::new();
        Solid::write_step([solid], &mut out).map_err(Self::err)?;
        Ok(out)
    }

    fn import_step(&self, bytes: &[u8]) -> Result<ShapeId, CadError> {
        let mut cur = Cursor::new(bytes);
        let solids = Solid::read_step(&mut cur).map_err(Self::err)?;
        let first = solids
            .into_iter()
            .next()
            .ok_or_else(|| CadError::Kernel("step contained no solids".into()))?;
        Ok(self.register(first))
    }

    fn export_stl(&self, s: ShapeId, params: TessParams) -> Result<Vec<u8>, CadError> {
        let table = self.table();
        let solid = table.get(&s.0).ok_or(CadError::UnknownShape(s))?;
        let mesh = Solid::mesh([solid], Self::tess(params)).map_err(Self::err)?;
        let mut out = Vec::new();
        mesh.write_stl(&mut out).map_err(Self::err)?;
        Ok(out)
    }

    fn clone_shape(&self, s: ShapeId) -> Result<ShapeId, CadError> {
        let mut table = self.table();
        let copy = table.get(&s.0).ok_or(CadError::UnknownShape(s))?.clone();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        table.insert(id, copy);
        Ok(ShapeId(id))
    }

    fn drop_shape(&self, s: ShapeId) {
        self.table().remove(&s.0);
    }
}
