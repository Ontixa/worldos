//! # worldos-cad
//!
//! CAD domain layer for WorldOS. [`CadKernel`] is the seam between the
//! command layer and a B-rep engine (production adapter: OCCT via
//! cadrum, see `docs/adr/0006-cad-engine.md`).
//!
//! Everything crossing the trait boundary is plain data — [`ShapeId`]
//! handles, byte payloads, and POD structs — so kernels stay
//! replaceable and the graph never holds kernel-native objects.

pub mod components;
pub mod error;
pub mod kernel;
pub mod tolerance;
pub mod types;

pub use components::{CAD_SCHEMA_VERSION, CadOperation, CadShape};
pub use error::CadError;
pub use kernel::CadKernel;
pub use tolerance::{
    ANGULAR_TOLERANCE_RAD, AREA_TOLERANCE_MM2, LINEAR_TOLERANCE_MM, RELATIVE_MEASURE_TOLERANCE,
    VOLUME_TOLERANCE_MM3, approx_mm, approx_relative,
};
pub use types::{BBox, BoolOp, Measures, MeshData, ShapeId, TessParams, Topology, TransformOp};
