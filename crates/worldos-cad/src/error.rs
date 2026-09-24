//! CAD errors — kernel-agnostic.

use thiserror::Error;

use crate::types::ShapeId;

#[derive(Debug, Error)]
pub enum CadError {
    #[error("kernel error: {0}")]
    Kernel(String),
    #[error("unknown shape handle {0:?}")]
    UnknownShape(ShapeId),
    #[error("operation not supported by this kernel: {0}")]
    Unsupported(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A selector resolved to zero elements where the caller needs at
    /// least one (e.g. an edge set for `cad.fillet`).
    #[error("selector `{selector}` matched no {target}")]
    SelectorEmpty {
        selector: String,
        target: &'static str,
    },
    /// A single-result selector (`top_face`, `face_extreme`, …) tied
    /// between several elements within tolerance.
    #[error("selector `{selector}` is ambiguous: {ids:?} all match within tolerance")]
    SelectorAmbiguous { selector: String, ids: Vec<u64> },
    /// A selector of one target kind was used where the other is
    /// required (e.g. a face selector as an edge set).
    #[error("selector targets {actual} but {expected} are required here")]
    SelectorKind {
        expected: &'static str,
        actual: &'static str,
    },
    /// `face_ids`/`edge_ids` referenced kernel ids that do not exist on
    /// the shape as loaded — the classic stale-reference failure after
    /// the source was regenerated.
    #[error(
        "selector references {target} ids not present on the shape: {ids:?} \
         (kernel topology ids are not stable across regeneration)"
    )]
    SelectorStale { ids: Vec<u64>, target: &'static str },
    /// Structurally malformed selector (bad direction, mixed-kind set
    /// expression, empty operand list, …).
    #[error("invalid selector: {0}")]
    BadSelector(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
