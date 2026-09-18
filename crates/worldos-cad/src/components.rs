//! `cad:*` component payloads — the persisted semantic state of CAD
//! objects in the project graph.
//!
//! Split of concerns (DECISIONS.md "parametric-first"):
//! - [`CadOperation`] is the **regeneration recipe** — authored state.
//! - [`CadShape`] is **derived state** — kernel outputs (artifact refs,
//!   measures, topology) that can always be rebuilt from the recipe.
//!
//! Component names live in `worldos_kernel::known::components` as
//! `CAD_OPERATION` / `CAD_SHAPE`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Measures, Topology};

/// Schema version embedded in every `cad:*` payload.
pub const CAD_SCHEMA_VERSION: u32 = 1;

/// `cad:operation` payload — the parametric recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CadOperation {
    pub v: u32,
    /// Operation kind, e.g. `create_box`, `boolean`, `fillet`,
    /// `transform`, `import_step`.
    pub kind: String,
    /// Kind-specific params (all lengths `*_mm`).
    pub params: Value,
    /// Kernel that must execute the recipe, e.g. `occt-8.0.1-cadrum`.
    /// Pinned so a project reopened under a different kernel fails
    /// loudly instead of silently producing different geometry.
    pub kernel: String,
}

impl CadOperation {
    pub fn new(kind: impl Into<String>, params: Value, kernel: impl Into<String>) -> Self {
        Self {
            v: CAD_SCHEMA_VERSION,
            kind: kind.into(),
            params,
            kernel: kernel.into(),
        }
    }
}

/// `cad:shape` payload — derived kernel output for one object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CadShape {
    pub v: u32,
    /// `sha256:<hex>` of the canonical BRep artifact — the source of
    /// truth for reopen/regeneration.
    pub brep: String,
    /// Optional `sha256:<hex>` of the exported STEP artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// Optional `sha256:<hex>` of the exported STL artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stl: Option<String>,
    /// Kernel that produced these outputs.
    pub kernel: String,
    /// Command type that produced this state, e.g. `cad.create_box`.
    pub generator: String,
    pub measures: Measures,
    pub topology: Topology,
}

impl CadShape {
    pub fn new(
        brep: impl Into<String>,
        kernel: impl Into<String>,
        generator: impl Into<String>,
        measures: Measures,
        topology: Topology,
    ) -> Self {
        Self {
            v: CAD_SCHEMA_VERSION,
            brep: brep.into(),
            step: None,
            stl: None,
            kernel: kernel.into(),
            generator: generator.into(),
            measures,
            topology,
        }
    }
}
