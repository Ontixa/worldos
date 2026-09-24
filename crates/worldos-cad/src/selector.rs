//! Semantic topology selectors — declarative expressions resolved
//! deterministically against a shape's CURRENT topology.
//!
//! A selector is plain serializable data (`{"op":"top_face"}`), so it
//! can live inside a `cad:operation` recipe and be re-resolved on every
//! replay (`cad.set_param` / `cad.regenerate`). That is the entire
//! stability contract: the *expression* is stable, the resolved kernel
//! ids are not. See `docs/cad-selectors.md` for the full contract.
//!
//! Resolution is pure data-in/data-out over a [`TopologyView`]: the
//! kernel reports faces/edges once, this module matches. Results are
//! sorted id sets — deterministic for a given view.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::CadError;
use crate::tolerance::{DIRECTION_DOT_TOLERANCE, LINEAR_TOLERANCE_MM};
use crate::types::{FaceSurface, TopologyView};

/// Which element kind a selector resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorTarget {
    Face,
    Edge,
}

impl SelectorTarget {
    /// Plural noun used in error messages (`"faces"` / `"edges"`).
    pub fn noun(self) -> &'static str {
        match self {
            Self::Face => "faces",
            Self::Edge => "edges",
        }
    }
}

/// A topology selector expression.
///
/// Wire form is a JSON object tagged by `op` (command layers may also
/// accept a bare string for the zero-argument ops, e.g. `"top_face"`):
///
/// ```json
/// {"op": "top_face"}
/// {"op": "faces_normal_to", "dir": [0, 0, 1]}
/// {"op": "edges_adjacent_to", "faces": {"op": "top_face"}}
/// {"op": "union", "of": [{"op": "top_face"}, {"op": "faces_of_kind", "kind": "cylinder"}]}
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Selector {
    // ---- face selectors ----
    /// Every face of the shape.
    AllFaces,
    /// Explicit kernel face ids — escape hatch. Load-scoped: ids are
    /// only meaningful for the loaded shape that produced them (cadrum
    /// reports `TShape` addresses); any id absent at resolution time
    /// is a [`CadError::SelectorStale`] failure.
    FaceIds { ids: Vec<u64> },
    /// Planar faces whose OUTWARD normal points within
    /// [`DIRECTION_DOT_TOLERANCE`] of `dir` (signed — `+Z` selects the
    /// top of a box, not the bottom). Curved faces never match: their
    /// normal field is position-dependent.
    FacesNormalTo { dir: [f64; 3] },
    /// Faces whose characteristic surface axis is parallel to `dir`
    /// (unsigned — sign is ignored): plane normals and
    /// cylinder/cone/torus axes. Selects e.g. a cylinder's lateral
    /// face AND its end caps for `dir = +Z`.
    FacesAxisTo { dir: [f64; 3] },
    /// Faces whose elementary surface is `kind` (`plane`, `cylinder`,
    /// `cone`, `sphere`, `torus`, `other`).
    FacesOfKind { kind: FaceSurface },
    /// The single face farthest along `dir`, scored by area-weighted
    /// center. Any surface kind competes. A tie within
    /// [`LINEAR_TOLERANCE_MM`] is a [`CadError::SelectorAmbiguous`]
    /// failure, not an arbitrary pick.
    FaceExtreme { dir: [f64; 3] },
    /// `top_face` — the highest planar face with outward normal `+Z`;
    /// among several, the one with the maximum z-center wins and a
    /// height tie is ambiguous. Empty on shapes with no upward planar
    /// face (e.g. a sphere).
    TopFace,
    /// `bottom_face` — [`Selector::TopFace`] mirrored to `-Z`.
    BottomFace,

    // ---- edge selectors ----
    /// Every edge of the shape.
    AllEdges,
    /// Explicit kernel edge ids — same staleness caveats as
    /// [`Selector::FaceIds`].
    EdgeIds { ids: Vec<u64> },
    /// Edges bounding at least one face matched by `faces` (outer and
    /// inner wires). `edges_adjacent_to(top_face)` on a box yields the
    /// four top edges.
    EdgesAdjacentTo { faces: Box<Selector> },
    /// All edges whose endpoint midpoint is within
    /// [`LINEAR_TOLERANCE_MM`] of the extreme along `dir` — a SET
    /// selector (e.g. all top edges of a box). Midpoint scoring is
    /// approximate for strongly curved edges; closed edges score at
    /// their coincident endpoints.
    EdgesExtreme { dir: [f64; 3] },

    // ---- set composition (all operands must share one target) ----
    /// Union of operand results.
    Union { of: Vec<Selector> },
    /// Intersection of operand results.
    Intersect { of: Vec<Selector> },
    /// Elements matching `base` but not `minus`.
    Difference {
        base: Box<Selector>,
        minus: Box<Selector>,
    },
}

impl Selector {
    /// The element kind this selector resolves to, checked recursively:
    /// set operands must agree on one kind, and `edges_adjacent_to`
    /// requires a face-valued operand.
    pub fn target(&self) -> Result<SelectorTarget, CadError> {
        use Selector::*;
        match self {
            AllFaces
            | FaceIds { .. }
            | FacesNormalTo { .. }
            | FacesAxisTo { .. }
            | FacesOfKind { .. }
            | FaceExtreme { .. }
            | TopFace
            | BottomFace => Ok(SelectorTarget::Face),
            AllEdges | EdgeIds { .. } | EdgesExtreme { .. } => Ok(SelectorTarget::Edge),
            EdgesAdjacentTo { faces } => {
                let inner = faces.target()?;
                if inner != SelectorTarget::Face {
                    return Err(CadError::SelectorKind {
                        expected: "faces",
                        actual: inner.noun(),
                    });
                }
                Ok(SelectorTarget::Edge)
            }
            Union { of } | Intersect { of } => {
                let (first, rest) = of.split_first().ok_or_else(|| {
                    CadError::BadSelector(format!(
                        "`{}` needs at least one operand",
                        self.op_name()
                    ))
                })?;
                let t = first.target()?;
                for s in rest {
                    let u = s.target()?;
                    if u != t {
                        return Err(CadError::BadSelector(format!(
                            "`{}` operands mix {} and {}",
                            self.op_name(),
                            t.noun(),
                            u.noun()
                        )));
                    }
                }
                Ok(t)
            }
            Difference { base, minus } => {
                let (b, m) = (base.target()?, minus.target()?);
                if b != m {
                    return Err(CadError::BadSelector(format!(
                        "`difference` operands mix {} and {}",
                        b.noun(),
                        m.noun()
                    )));
                }
                Ok(b)
            }
        }
    }

    /// The `op` tag of this selector's root node (for diagnostics).
    pub fn op_name(&self) -> &'static str {
        use Selector::*;
        match self {
            AllFaces => "all_faces",
            FaceIds { .. } => "face_ids",
            FacesNormalTo { .. } => "faces_normal_to",
            FacesAxisTo { .. } => "faces_axis_to",
            FacesOfKind { .. } => "faces_of_kind",
            FaceExtreme { .. } => "face_extreme",
            TopFace => "top_face",
            BottomFace => "bottom_face",
            AllEdges => "all_edges",
            EdgeIds { .. } => "edge_ids",
            EdgesAdjacentTo { .. } => "edges_adjacent_to",
            EdgesExtreme { .. } => "edges_extreme",
            Union { .. } => "union",
            Intersect { .. } => "intersect",
            Difference { .. } => "difference",
        }
    }

    /// Compact rendering for error messages and receipts.
    pub fn describe(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{self:?}"))
    }
}

/// Resolve `sel` against `view` in the selector's own target kind.
/// Returns the sorted, deduplicated kernel id set — possibly empty
/// (callers that require a match turn it into
/// [`CadError::SelectorEmpty`]).
pub fn resolve(sel: &Selector, view: &TopologyView) -> Result<BTreeSet<u64>, CadError> {
    match sel.target()? {
        SelectorTarget::Face => resolve_faces(sel, view),
        SelectorTarget::Edge => resolve_edges(sel, view),
    }
}

/// Resolve `sel`, which must be face-targeted.
pub fn resolve_faces(sel: &Selector, view: &TopologyView) -> Result<BTreeSet<u64>, CadError> {
    if sel.target()? == SelectorTarget::Edge {
        return Err(CadError::SelectorKind {
            expected: "faces",
            actual: "edges",
        });
    }
    resolve_faces_inner(sel, view)
}

/// Resolve `sel`, which must be edge-targeted.
pub fn resolve_edges(sel: &Selector, view: &TopologyView) -> Result<BTreeSet<u64>, CadError> {
    if sel.target()? == SelectorTarget::Face {
        return Err(CadError::SelectorKind {
            expected: "edges",
            actual: "faces",
        });
    }
    resolve_edges_inner(sel, view)
}

fn resolve_faces_inner(sel: &Selector, view: &TopologyView) -> Result<BTreeSet<u64>, CadError> {
    use Selector::*;
    match sel {
        AllFaces => Ok(view.faces.iter().map(|f| f.id).collect()),
        FaceIds { ids } => check_present(ids, view.faces.iter().map(|f| f.id), "faces"),
        FacesNormalTo { dir } => {
            let d = unit(*dir)?;
            Ok(view
                .faces
                .iter()
                .filter(|f| {
                    f.normal
                        .is_some_and(|n| cosine(n, d) >= 1.0 - DIRECTION_DOT_TOLERANCE)
                })
                .map(|f| f.id)
                .collect())
        }
        FacesAxisTo { dir } => {
            let d = unit(*dir)?;
            Ok(view
                .faces
                .iter()
                .filter(|f| {
                    f.axis
                        .is_some_and(|a| cosine(a, d).abs() >= 1.0 - DIRECTION_DOT_TOLERANCE)
                })
                .map(|f| f.id)
                .collect())
        }
        FacesOfKind { kind } => Ok(view
            .faces
            .iter()
            .filter(|f| f.surface == *kind)
            .map(|f| f.id)
            .collect()),
        FaceExtreme { dir } => {
            let d = unit(*dir)?;
            extreme(view.faces.iter().collect(), d, sel)
        }
        TopFace => {
            let d = [0.0, 0.0, 1.0];
            let cands = upward(view, d);
            extreme(cands, d, sel)
        }
        BottomFace => {
            let d = [0.0, 0.0, -1.0];
            let cands = upward(view, d);
            extreme(cands, d, sel)
        }
        Union { of } => {
            let mut acc = BTreeSet::new();
            for s in of {
                acc.extend(resolve_faces_inner(s, view)?);
            }
            Ok(acc)
        }
        Intersect { of } => {
            let mut it = of.iter();
            let mut acc = resolve_faces_inner(it.next().expect("non-empty via target()"), view)?;
            for s in it {
                let other = resolve_faces_inner(s, view)?;
                acc = acc.intersection(&other).copied().collect();
            }
            Ok(acc)
        }
        Difference { base, minus } => {
            let mut acc = resolve_faces_inner(base, view)?;
            for id in resolve_faces_inner(minus, view)? {
                acc.remove(&id);
            }
            Ok(acc)
        }
        edge_op => Err(CadError::SelectorKind {
            expected: "faces",
            actual: match edge_op.target() {
                Ok(t) => t.noun(),
                Err(_) => "unknown",
            },
        }),
    }
}

fn resolve_edges_inner(sel: &Selector, view: &TopologyView) -> Result<BTreeSet<u64>, CadError> {
    use Selector::*;
    match sel {
        AllEdges => Ok(view.edges.iter().map(|e| e.id).collect()),
        EdgeIds { ids } => check_present(ids, view.edges.iter().map(|e| e.id), "edges"),
        EdgesAdjacentTo { faces } => {
            let matched = resolve_faces_inner(faces, view)?;
            let universe: BTreeSet<u64> = view.edges.iter().map(|e| e.id).collect();
            Ok(view
                .faces
                .iter()
                .filter(|f| matched.contains(&f.id))
                .flat_map(|f| f.edge_ids.iter().copied())
                .filter(|id| universe.contains(id))
                .collect())
        }
        EdgesExtreme { dir } => {
            let d = unit(*dir)?;
            let best = view
                .edges
                .iter()
                .map(|e| edge_score(e, d))
                .fold(f64::NEG_INFINITY, f64::max);
            if best == f64::NEG_INFINITY {
                return Ok(BTreeSet::new());
            }
            Ok(view
                .edges
                .iter()
                .filter(|e| best - edge_score(e, d) <= LINEAR_TOLERANCE_MM)
                .map(|e| e.id)
                .collect())
        }
        Union { of } => {
            let mut acc = BTreeSet::new();
            for s in of {
                acc.extend(resolve_edges_inner(s, view)?);
            }
            Ok(acc)
        }
        Intersect { of } => {
            let mut it = of.iter();
            let mut acc = resolve_edges_inner(it.next().expect("non-empty via target()"), view)?;
            for s in it {
                let other = resolve_edges_inner(s, view)?;
                acc = acc.intersection(&other).copied().collect();
            }
            Ok(acc)
        }
        Difference { base, minus } => {
            let mut acc = resolve_edges_inner(base, view)?;
            for id in resolve_edges_inner(minus, view)? {
                acc.remove(&id);
            }
            Ok(acc)
        }
        face_op => Err(CadError::SelectorKind {
            expected: "edges",
            actual: match face_op.target() {
                Ok(t) => t.noun(),
                Err(_) => "unknown",
            },
        }),
    }
}

/// Faces whose outward normal matches `d` — the candidate pool for
/// `top_face`/`bottom_face`.
fn upward(view: &TopologyView, d: [f64; 3]) -> Vec<&crate::types::FaceDetail> {
    view.faces
        .iter()
        .filter(|f| {
            f.normal
                .is_some_and(|n| cosine(n, d) >= 1.0 - DIRECTION_DOT_TOLERANCE)
        })
        .collect()
}

/// Single-result extreme: the face whose center is farthest along `d`.
/// A tie within [`LINEAR_TOLERANCE_MM`] is ambiguous; an empty
/// candidate pool yields an empty set (callers decide whether that is
/// an error).
fn extreme(
    cands: Vec<&crate::types::FaceDetail>,
    d: [f64; 3],
    sel: &Selector,
) -> Result<BTreeSet<u64>, CadError> {
    let best = cands
        .iter()
        .map(|f| dot(f.center_mm, d))
        .fold(f64::NEG_INFINITY, f64::max);
    if best == f64::NEG_INFINITY {
        return Ok(BTreeSet::new());
    }
    let tied: Vec<u64> = cands
        .iter()
        .filter(|f| best - dot(f.center_mm, d) <= LINEAR_TOLERANCE_MM)
        .map(|f| f.id)
        .collect();
    if tied.len() > 1 {
        return Err(CadError::SelectorAmbiguous {
            selector: sel.describe(),
            ids: tied,
        });
    }
    Ok(tied.into_iter().collect())
}

/// `ids` must all exist in `universe` — a missing kernel id is a stale
/// reference (the shape was rebuilt or re-loaded since the id was
/// captured).
fn check_present(
    ids: &[u64],
    universe: impl Iterator<Item = u64>,
    target: &'static str,
) -> Result<BTreeSet<u64>, CadError> {
    let universe: BTreeSet<u64> = universe.collect();
    let missing: Vec<u64> = ids
        .iter()
        .copied()
        .filter(|id| !universe.contains(id))
        .collect();
    if !missing.is_empty() {
        return Err(CadError::SelectorStale {
            ids: missing,
            target,
        });
    }
    Ok(ids.iter().copied().collect())
}

/// Edge extreme score: midpoint of the endpoints, projected on `d`.
fn edge_score(e: &crate::types::EdgeDetail, d: [f64; 3]) -> f64 {
    let mid = [
        (e.start_mm[0] + e.end_mm[0]) / 2.0,
        (e.start_mm[1] + e.end_mm[1]) / 2.0,
        (e.start_mm[2] + e.end_mm[2]) / 2.0,
    ];
    dot(mid, d)
}

/// Validate `dir` is a finite non-zero vector and return it normalized.
fn unit(dir: [f64; 3]) -> Result<[f64; 3], CadError> {
    let l = dot(dir, dir).sqrt();
    if !l.is_finite() || l <= 0.0 {
        return Err(CadError::BadSelector(
            "direction must be a finite non-zero vector".into(),
        ));
    }
    Ok([dir[0] / l, dir[1] / l, dir[2] / l])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cosine of the angle between two vectors; `0.0` when either is
/// degenerate (never matches a direction tolerance check).
fn cosine(a: [f64; 3], b: [f64; 3]) -> f64 {
    let la = dot(a, a).sqrt();
    let lb = dot(b, b).sqrt();
    if la <= 0.0 || lb <= 0.0 {
        return 0.0;
    }
    (dot(a, b) / (la * lb)).clamp(-1.0, 1.0)
}
