# Limitations

Brutally honest current-state constraints. Updated when reality changes.

## Security

- **Native plugins are trusted local code.** `worldos-plugin-*`
  subprocesses run with the host user's privileges. Manifest
  `permissions` narrow the *WorldOS* actor (what commands it may run)
  but do NOT sandbox the process: a plugin can still read files, spawn
  processes, and use the network regardless of the manifest. Real
  isolation requires the WASM plugin path (not built yet).
- **No transport auth.** JSON-RPC (stdio + WebSocket) and MCP expose
  every command the serving actor permits. Bind to localhost only; any
  connected client is the local actor.
- **`.worldos` files are trust boundaries.** Opening a project replays
  its history; treat files from untrusted sources as executable
  documents.

## Persistence & recovery

- **No crash-injection test suite yet.** Saves are atomic by
  construction (single SQLite transaction + WAL), but restart-after-
  kill recovery is not systematically proven.
- **Migration coverage is thin.** Only schema v1 exists; no
  historical-version fixture matrix.
- **CAD artifacts require a sidecar.** BRep/STEP/STL outputs are stored in
  `<project>.artifacts/`; copying only the SQLite file is insufficient.
  Undo restores graph references, not deletion of artifact blobs or arbitrary
  filesystem effects. See [CAD CLI workflow](../cad-cli.md).

## Geometry

- **Analytic and native geometry are distinct.** `geom:*` objects still
  measure from component parameters; `geometry.measure` is not kernel-verified.
  Native `cad:*` bodies use the optional cadrum/OCCT adapter. The CLI requires
  both a `cad` feature build and explicit `--cad` attachment; the desktop does
  not attach this backend. Native CAD is not a universal modeling guarantee.
- **Rotation ignored by measure.** `object_dims` applies scale but not
  rotation — bbox/volume are correct for volume (rotation-invariant)
  but `object_bbox` is wrong for rotated objects.
- **Mesh export is a faceted approximation.** `geometry.export`
  tessellates the analytic primitives with fixed segment counts
  (`kernel::mesh`): spheres/cylinders/cones/tori come out as inscribed
  polygonal solids (~1–3% under analytic volume), `plane` exports an
  open two-triangle surface, and rotation is not applied (same caveat
  as `object_bbox`). `cad:body` B-rep fidelity is out of scope here —
  use `cad.export_stl`. Exported files are external effects outside
  undo; the attributed command record (with content digest) is the
  audit trail.
- **Mesh import is triangles, not topology.** `geometry.import` reads
  binary STL and Wavefront OBJ into a `geom:mesh` component
  (`positions`/`indices`, vertex-welded for STL). There is no healing,
  manifold repair, or B-rep reconstruction: open meshes measure ~0
  volume by the divergence theorem, inverted winding yields |volume|,
  and holes stay holes. The mesh is stored inline in the project file,
  so imports are bounded to 100k triangles / 64 MiB sources; huge
  scans need the artifact store + `cad:body` path instead. ASCII STL,
  facet normals, UVs, and OBJ materials are ignored — winding order is
  preserved as authored.

## Agent

- **Goal verification needs a declared predicate.** The tool-use loop
  (`AgentRuntime::run_spec`) iterates observe → plan → act → inspect
  until `done_when` — an expression in the requirement grammar —
  verifies true on live in-transaction state, or fails honestly and
  rolls back. Natural-language goals do NOT auto-derive a predicate:
  `RulePlanner` cannot synthesize one and the LLM planner only *proposes
  steps* — a caller that omits `done_when` gets the legacy single-pass
  semantics where success means "the plan executed and references check
  out", not that the objective is semantically true.
- **Predicate expressiveness is the requirement grammar's.** `exists`,
  `exists_named`, `count`, `volume`, `area`, `distance`,
  `object(name).component.path` + boolean connectives/comparisons —
  enough for presence/cardinality/measure goals; not arbitrary
  postconditions (no relation-shape queries, no negated-existence edge
  cases beyond `not exists_named`). An unevaluable expression (bad term,
  missing object in a measure term) reads as "unsatisfied" each round —
  the run fails at the iteration cap or a planner dead-end with the
  last verdict in the report.
- **Reference checks are presence checks.** Every top-level output `id`
  must still exist after its iteration and at commit; `relation.add`
  outputs are checked as relations. Malformed or absent references fail
  the run and trigger rollback, retaining command outputs in the failure
  report. Commands without an `id` output have no reference check; this
  is not full postcondition verification.
- **Stateless planners can churn.** `RulePlanner` replans the same goal
  each iteration, so an unsatisfiable `done_when` burns commands until
  `max_iterations`/`max_commands` trip — bounded, honest, but not smart.
  Only `LlmPlanner` currently uses the between-iteration observation.
- **LLM planner is BYOK-only** (`WORLDOS_LLM_*`); no bundled provider.

## Scale & platform

- **Performance unmeasured.** No benchmarks; no validated object-count
  ceiling; `find_by_name`/relation scans are O(n).
- **Windows-only CI** (by design for now — `windows-latest`). Linux/
  macOS toolchains are untested; desktop is MinGW-checked only
  (cdylib workaround in `1087e28`).
- **Desktop is a thin viewer** — no real CAD viewport, no diff view,
  no transaction preview.

## Testing

- 36 tests cover the golden path. No property tests, no fuzzing, no
  malformed-input campaigns, no migration matrix.
