# Engineering Status

Only demonstrably working behavior is listed here. Verified on
2026-09-19 (Windows 10, `x86_64-pc-windows-gnu`, cargo test
--workspace green; WorldBench corpus 6/6 pass).

## Working today

- **UPG kernel** — objects, schema-versioned components, typed relations,
  actors, permission sets, `core:contains` containment, `core:depends-on`
  dependency tracking, search.
- **Commands** — 25 builtin handlers (object/relation/document/code/
  geometry/meta/requirement/decision). Schema-validated, permission-checked,
  composable via `ctx.run_sub`.
- **Transactions** — atomic commit/rollback of `StateOp` groups; linear
  undo/redo cursor; redo-tail truncation on new writes.
- **History** — attributed `TransactionRecord`s persisted per project.
- **Persistence** — `.worldos` SQLite (WAL, `synchronous=FULL`), full
  snapshot + history in one file, `user_version` migrations (v1 only so
  far). Atomic rewrite on save.
- **Requirement staleness** — a write to a depended-on object flips
  dependent `core:requirement` status to `stale` in the same transaction.
- **Requirement expressions** — `and`/`or`/`not`/parens grammar; measure
  terms `volume(x)`, `area(x)`, `distance(a,b)` over analytic primitives
  and imported `geom:mesh` objects.
- **Geometry (analytic)** — `geometry.create_primitive` for
  cube/sphere/cylinder/cone/torus/plane; `geometry.transform`;
  `geometry.measure` capability (bbox, volume, surface area);
  `geometry.export` command + capability — deterministic tessellation
  (`kernel::mesh`) to binary STL / OBJ files, `..`-guarded,
  never-overwrite, `filesystem.write`-gated; `geometry.import`
  command + capability — binary STL / OBJ → `geom:mesh` objects
  (`kernel::mesh_import`, vertex-welded, ≤100k triangles, ≤64 MiB,
  `filesystem.read`-gated), measurable and re-exportable. Primitives are
  **analytic approximations from component data — not a B-rep kernel**;
  imported meshes are triangles, not topology.
- **Capabilities** — registry + permission guard + `CapabilityHost`;
  `plugin.run` exposes hosted plugins.
- **Plugin runtime** — hosted `worldos-plugin-*` subprocesses speaking
  line-delimited JSON-RPC over stdio; one `plugin:<name>` transaction per
  session; commit on clean exit, rollback on crash/timeout/protocol
  violation; sidecar `.json` manifest grants exact permissions.
- **Agent runtime** — bounded observe → plan → act → inspect loop inside
  one transaction (`AgentRuntime::run_spec`): an optional `done_when`
  predicate in the requirement-expression grammar is evaluated against
  live state every iteration and is the only success exit; `Budget`
  caps iterations (default 8) and total commands (default 32), both
  overridable per run. `Planner::plan_turn` receives an `Observation`
  (prior step records, last verdict, remaining budget) so planners can
  replan — `RulePlanner`, `LlmPlanner` (feature `llm`, OpenAI-compatible
  BYOK; prompt carries the observation), `FallbackPlanner`; per-run
  permission profiles. Reported object and relation IDs are checked for
  presence after each iteration and again before commit; invalid/missing
  references fail and trigger rollback, preserving step evidence. With
  no `done_when` the run is one bounded plan→act→verify pass — success
  still means "commands ran and refs check out", not that the goal is
  semantically true (see `LIMITATIONS.md`).
- **Interfaces** — CLI (`worldos`), JSON-RPC over stdio + WebSocket, MCP
  server, TypeScript SDK, Python SDK (stdlib-only), Tauri 2 desktop
  (builds; viewport renders analytic primitives).
- **CI** — windows-latest: fmt + clippy + tests; TS+Python SDK build;
  desktop `cargo check`.
- **Artifacts** — `worldos-artifact` content-addressed store
  (SHA-256, fan-out `objects/<hh>/<hex>`, atomic writes, verify-on-read,
  gc); sidecar `<project>.artifacts/` follows `save_as`.
- **Real CAD (Forge slice 1)** — `worldos-cad` `CadKernel` trait +
  `worldos-adapter-cadrum` (OCCT 8.0.1). Commands: `cad.create_{box,
  cylinder,sphere}`, `cad.boolean`, `cad.fillet`, `cad.chamfer`,
  `cad.transform`, `cad.measure`, `cad.export_{step,stl}`,
  `cad.import_step`, `cad.set_param`, `cad.regenerate`. `cad:operation`
  recipes are replayable; `core:derived-from` edges form the feature
  tree; `cad:shape.stale` flags dependents. `position` is baked into
  the BRep (world-space truth).
- **WorldBench v0** — `worldos-bench` crate + `bench/tasks/*.yaml`
  corpus (6 tasks) + `bench/reports/v0-baseline.json` (6/6 pass).
- **Adversarial property tests** (`proptest`) — random command
  sequences prove undo-all restores initial state and redo-all
  restores final; failed commands inside explicit transactions leave
  no graph or journal residue; random snapshots survive
  save→reopen byte-exact (project + history); spawned-process
  `abort()` mid-write and post-commit recovers the last committed
  snapshot (`worldos-store/tests/crash_recovery.rs`).

## Verified external dependency

- **cadrum 0.8.20** (static OCCT 8.0.1, `x86_64-pc-windows-gnu` prebuilt)
  verified 2026-09-18: cube/cylinder, boolean fuse/cut/intersect,
  fillet_edges, chamfer_edges, `read_step`/`write_step` round-trip
  (vol stable at 1e-4), `read_brep`/`write_brep` round-trip,
  `Mesh::write_stl`, `Solid::mesh` tessellation, `iter_edge`/`iter_face`
  with stable `id()`s. See `docs/adr/0006-cad-engine.md`.

## Not yet working (see LIMITATIONS.md)

- Semantic CAD topology selectors (`top_face`, `edges_adjacent_to`) —
  `edge_ids`/`face_ids` in `cad:shape.topology` are raw kernel ids.
- Deep regen: `cad.regenerate` replays one node using sources' current
  BReps; no topological replay of a stale chain yet.
- Fuzz targets (requirement parser, StateOp streams, JSON-RPC, plugin
  protocol, migration input) and in-process I/O-fault injection.
- Plugin sandboxing (native plugins are trusted local code).
- Collaboration / multi-writer.
