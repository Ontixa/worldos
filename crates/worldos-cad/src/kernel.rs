//! `CadKernel`: the seam between WorldOS commands and a B-rep engine.
//!
//! Every method is session-scoped: [`ShapeId`]s are opaque handles owned
//! by the kernel. Persisted state crosses the boundary only as bytes
//! (BRep/STEP/STL) or plain data (measures, topology, meshes) — the
//! graph never sees a kernel-native object.

use crate::error::CadError;
use crate::types::{BoolOp, Measures, MeshData, ShapeId, TessParams, Topology, TransformOp};

pub trait CadKernel: Send + Sync {
    /// Stable kernel identifier, e.g. `occt-8.0.1-cadrum` — recorded in
    /// `cad:shape.kernel` for provenance.
    fn name(&self) -> &'static str;

    // ---- primitives (all dims mm) ----
    /// Axis-aligned box from origin to (sx, sy, sz).
    fn make_box(&self, sx_mm: f64, sy_mm: f64, sz_mm: f64) -> Result<ShapeId, CadError>;
    /// Cylinder along +Z, base at origin.
    fn make_cylinder(&self, radius_mm: f64, height_mm: f64) -> Result<ShapeId, CadError>;
    fn make_sphere(&self, radius_mm: f64) -> Result<ShapeId, CadError>;

    // ---- operations (each returns a NEW handle; inputs stay alive) ----
    fn boolean(&self, a: ShapeId, b: ShapeId, op: BoolOp) -> Result<ShapeId, CadError>;
    /// `edges` are kernel topology ids from [`Topology::edge_ids`];
    /// an empty slice means "all edges".
    fn fillet(&self, s: ShapeId, radius_mm: f64, edges: &[u64]) -> Result<ShapeId, CadError>;
    fn chamfer(&self, s: ShapeId, distance_mm: f64, edges: &[u64]) -> Result<ShapeId, CadError>;
    /// Apply transform steps in order; returns a new handle.
    fn transform(&self, s: ShapeId, ops: &[TransformOp]) -> Result<ShapeId, CadError>;

    // ---- inspection ----
    fn measure(&self, s: ShapeId) -> Result<Measures, CadError>;
    fn topology(&self, s: ShapeId) -> Result<Topology, CadError>;
    fn mesh(&self, s: ShapeId, params: TessParams) -> Result<MeshData, CadError>;

    // ---- serialization (bytes; callers persist via artifact store) ----
    fn export_brep(&self, s: ShapeId) -> Result<Vec<u8>, CadError>;
    fn import_brep(&self, bytes: &[u8]) -> Result<ShapeId, CadError>;
    fn export_step(&self, s: ShapeId) -> Result<Vec<u8>, CadError>;
    fn import_step(&self, bytes: &[u8]) -> Result<ShapeId, CadError>;
    fn export_stl(&self, s: ShapeId, params: TessParams) -> Result<Vec<u8>, CadError>;

    // ---- handle lifecycle ----
    /// Deep copy — independent handle to the same geometry.
    fn clone_shape(&self, s: ShapeId) -> Result<ShapeId, CadError>;
    /// Release the handle. Unknown ids are ignored (idempotent).
    fn drop_shape(&self, s: ShapeId);
}
