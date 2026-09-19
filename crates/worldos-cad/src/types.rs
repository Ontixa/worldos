//! Kernel-agnostic CAD value types.
//!
//! Units: all lengths are **millimeters**, all volumes **mm³**, all
//! areas **mm²** (see docs/engineering/DECISIONS.md — `*_mm` suffixes
//! are mandatory in persisted component data; these in-memory types
//! follow the same convention).

use serde::{Deserialize, Serialize};

/// Opaque kernel-scoped handle to a live B-rep shape. Handles are
/// session-local: they are invalidated when the kernel is dropped and
/// are NOT stable across save/reopen (persistence goes through BRep
/// artifacts, never handles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShapeId(pub u64);

/// Boolean operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
}

/// Axis-aligned bounding box, mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
}

/// Kernel-verified measures of a shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measures {
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub bbox: BBox,
    pub center_mm: [f64; 3],
}

/// Geometric descriptor of one edge — the basis of position-based
/// selection. `id` is a geometry-derived identity (hash of quantized
/// endpoints, arc-length midpoint and length): stable across BRep
/// round-trips and across regeneration *while the edge's geometry is
/// unchanged*. If a rebuild moves or removes the edge the id changes —
/// that is exactly what stale-selection detection needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeInfo {
    pub id: u64,
    pub length_mm: f64,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub mid_mm: [f64; 3],
}

/// Topology census + validity summary of a shape.
///
/// `is_valid` is an operational definition, not a full BRepCheck
/// analysis: non-null solid with positive volume. Kernels that expose
/// a real analyzer may tighten it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    pub solids: u32,
    pub faces: u32,
    pub edges: u32,
    pub is_solid: bool,
    pub is_valid: bool,
    /// Geometry-derived edge identities (see [`EdgeInfo::id`]) — usable
    /// as fillet/chamfer selectors across sessions and regenerations.
    pub edge_ids: Vec<u64>,
    /// Kernel topology ids of the shape's faces (session-scoped).
    pub face_ids: Vec<u64>,
    /// Per-edge geometric descriptors backing `edge_ids`.
    #[serde(default)]
    pub edges_detail: Vec<EdgeInfo>,
}

/// A single rigid-body / affine transform step, applied in order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransformOp {
    /// Move by (dx, dy, dz) mm.
    Translate { delta_mm: [f64; 3] },
    /// Rotate `angle_rad` around the axis through `origin_mm` along
    /// `dir` (need not be normalized).
    RotateAxis {
        origin_mm: [f64; 3],
        dir: [f64; 3],
        angle_rad: f64,
    },
    /// Uniform scale about `center_mm`.
    Scale { center_mm: [f64; 3], factor: f64 },
}

/// Tessellation controls for [`crate::CadKernel::mesh`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TessParams {
    /// Max chord-to-surface distance; absolute mm unless `relative`.
    pub deflection_linear: f64,
    /// Max angle between adjacent facet directions, radians.
    pub deflection_angular: f64,
    /// Interpret `deflection_linear` relative to local feature size.
    pub relative: bool,
}

impl Default for TessParams {
    fn default() -> Self {
        Self {
            deflection_linear: 0.004,
            deflection_angular: 0.5,
            relative: true,
        }
    }
}

/// Triangulated shape data, mm. `indices` are triangle corners into
/// `positions`/`normals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshData {
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub indices: Vec<u32>,
    /// Kernel face id per triangle — enables face-level selectors.
    pub face_ids: Vec<u64>,
}
