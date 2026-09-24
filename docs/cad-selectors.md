# Semantic topology selectors for CAD

`cad:*` commands need to refer to faces and edges — "the top face", "the
edges around that face" — without hard-coding kernel topology ids, which
change every time a shape is rebuilt *or even re-loaded* (the cadrum
kernel reports OCCT `TShape` addresses — pointers scoped to one loaded
solid). A **selector** is a small declarative expression that resolves
to a set of kernel ids against the shape's *current* topology.

Selectors are plain JSON (`{"op":"top_face"}`), so they can be stored in
`cad:operation` recipes and re-resolved on every parametric replay
(`cad.set_param` / `cad.regenerate`). Resolution is deterministic: same
shape, same selector, same id set — sorted ascending, duplicates
removed.

## Using selectors

- `cad.fillet` / `cad.chamfer` accept `edge_select` (in addition to the
  legacy raw `edge_ids` array; both present = union of both matches).
- `cad.measure` accepts `select` — resolved ids land in the output as
  `selection: {kind, matched, ids, area_mm2?}` (`area_mm2` is the sum of
  matched face areas; face selections only).
- `cad.select` is a pure inspection command —
  `{"object": "block", "select": <expr>}` returns the matched ids plus
  per-element detail (`faces[]` or `edges[]` entries with centers,
  normals, surface kinds, boundary edge ids). Use it to preview a
  selector before committing it to a recipe.

A bare string is shorthand for a zero-argument op: `"top_face"` ≡
`{"op":"top_face"}`.

## Selector grammar

Target kinds: a selector resolves to **faces** or **edges**. Using a
face selector where edges are required (and vice versa) is a
`SelectorKind` error. `edges_adjacent_to` bridges the two: it is an
edge selector whose operand is a face selector.

### Face selectors

| op | args | matches |
|----|------|---------|
| `all_faces` | — | every face |
| `face_ids` | `ids: [u64]` | explicit kernel ids — escape hatch, **load-scoped** (see the stability contract) |
| `faces_normal_to` | `dir: [x,y,z]` | planar faces whose **outward** normal points within `DIRECTION_DOT_TOLERANCE` (≈0.08°) of `dir` — signed, `+Z` ≠ `-Z`. Curved faces never match: their normal varies across the face |
| `faces_axis_to` | `dir` | faces whose characteristic axis is parallel to `dir`, **unsigned**: plane normals and cylinder/cone/torus axes. On a cylinder `+Z` matches the lateral face *and* both caps |
| `faces_of_kind` | `kind` | faces whose elementary surface is `plane`/`cylinder`/`cone`/`sphere`/`torus`/`other` (`other` = B-spline, offset, anything non-elementary) |
| `face_extreme` | `dir` | the single face farthest along `dir`, scored by area-weighted center. Any surface kind competes. A tie within `LINEAR_TOLERANCE_MM` is `SelectorAmbiguous`, never a silent pick |
| `top_face` | — | the highest planar face with outward normal `+Z`; several → the max-z one; a height tie → ambiguous. Empty when no upward planar face exists (a sphere has none) |
| `bottom_face` | — | `top_face` mirrored to `-Z` |

### Edge selectors

| op | args | matches |
|----|------|---------|
| `all_edges` | — | every edge |
| `edge_ids` | `ids: [u64]` | explicit kernel ids — same load-scoped staleness caveat as `face_ids` |
| `edges_adjacent_to` | `faces: <face selector>` | edges bounding at least one matched face (outer and inner wires). `edges_adjacent_to(top_face)` on a box = the 4 top edges |
| `edges_extreme` | `dir` | all edges whose endpoint midpoint is within `LINEAR_TOLERANCE_MM` of the extreme along `dir` — a set, e.g. all top edges of a box. Midpoint scoring is approximate on strongly curved edges |

### Set composition

`{"op":"union","of":[…]}`, `{"op":"intersect","of":[…]}`,
`{"op":"difference","base":…,"minus":…}` — all operands must share one
target kind; a mixed union is `BadSelector`.

## Stability contract

What a selector guarantees across parametric regeneration
(`cad.set_param` → `cad.regenerate`, or reopen + replay):

- **Re-resolution, not identity.** Recipes store the selector
  *expression*. Every replay resolves it against the source's current
  BRep. If the geometry still answers the description — the top of a
  resized box is still its top — the feature follows: the same recipe
  fillets the new box's top edges.
- **Determinism.** For a fixed topology view the result is a sorted id
  set. Two commands running the same selector on the same shape agree.
- **Fail-closed absence.** If the selected geometry is gone (a boolean
  removed the top face), resolution returns empty and the command fails
  — `regenerate` rolls back, the stale `cad:shape` is left untouched.

What it does **not** guarantee:

- **Stable ids.** `face_ids`/`edge_ids` pin raw kernel ids, which are
  *load-scoped*: with the cadrum kernel an id is the address of an
  OCCT `TShape`, and every command re-imports the BRep — so an id
  captured by `cad.measure` or `cad.select` is already dead by the
  time `cad.fillet` runs, and a recipe storing raw ids fails
  `SelectorStale` on replay. (Worse than absent, a recycled address
  could silently name a *different* element — resolution cannot detect
  that; treat raw ids as advisory-only.) `SelectorStale` on a missing
  id is a deliberate, honest failure — use a semantic selector for
  anything meant to survive a command boundary.
- **Stable match count.** Parametric edits can add or remove matches:
  `faces_normal_to(+Z)` matching one face today may match two after a
  union adds a step — the feature applies to *both*. Set-valued
  selectors apply to whatever matches; cardinality changes are applied,
  not warned about.
- **Semantic intent.** Selectors are geometric predicates, not names.
  `top_face` does not know which face you "meant" — after a transform
  rotates the body, `top_face` picks whatever is now on top. There is
  no persistent face naming.
- **Curved-face normals.** `faces_normal_to`/`top_face`/`bottom_face`
  only see planes (and exactly their outward orientation). Use
  `faces_axis_to` or `faces_of_kind` for cylinders/cones/tori.
- **`edges_extreme` on curved edges** scores endpoints' midpoint — a
  strongly arched edge whose apex exceeds its endpoints may not be the
  reported extreme.

## Errors

All selector failures are structured `CadError` variants surfaced as
command errors (the transaction rolls back; nothing mutates):

| variant | meaning |
|---------|---------|
| `SelectorEmpty` | required at least one match, got none (e.g. a fillet edge set) |
| `SelectorAmbiguous` | a single-result selector (`top_face`, `face_extreme`) tied within tolerance — the error lists the tying ids |
| `SelectorKind` | face selector used where edges are required, or vice versa |
| `SelectorStale` | `face_ids`/`edge_ids` named ids absent from the current topology — the shape was rebuilt *or re-loaded* since the ids were captured (kernel ids are load-scoped) |
| `BadSelector` | malformed expression: zero direction, mixed-kind set, empty operand list |

`cad.select` reports `matched: 0` for an empty result instead of
erroring — it is a probe; `SelectorEmpty` exists for contexts that need
geometry (a fillet of nothing is not a fillet).

## Implementation notes

Resolution is kernel-agnostic: `worldos_cad::selector` evaluates
expressions over a `TopologyView` produced by
`CadKernel::topology_view` — per-face `{id, center_mm, normal?, axis?,
surface, edge_ids, area_mm2}` and per-edge `{id, start_mm, end_mm}`.
The cadrum adapter reports outward normals for planes via
`Face::project(center)` and surface axes from the elementary-surface
placement; curved faces report no single normal (see above). The
tolerances live in `worldos_cad::tolerance`: `DIRECTION_DOT_TOLERANCE`
for normal/axis alignment, `LINEAR_TOLERANCE_MM` for extreme ties.
