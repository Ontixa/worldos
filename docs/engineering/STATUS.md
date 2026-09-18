# Engineering Status

Only demonstrably working behavior is listed here. Verified against
`main @ 0c7e480` on 2026-09-18 (Windows 10, `x86_64-pc-windows-gnu`,
cargo test --workspace: 36 tests green).

## Working today

- **UPG kernel** — objects, schema-versioned components, typed relations,
  actors, permission sets, `core:contains` containment, `core:depends-on`
  dependency tracking, search.
- **Commands** — 23 builtin handlers (object/relation/document/code/
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
  terms `volume(x)`, `area(x)`, `distance(a,b)` over analytic primitives.
- **Geometry (analytic)** — `geometry.create_primitive` for
  cube/sphere/cylinder/cone/torus/plane; `geometry.transform`;
  `geometry.measure` capability (bbox, volume, surface area). These are
  **analytic approximations from component data — not a B-rep kernel.**
- **Capabilities** — registry + permission guard + `CapabilityHost`;
  `plugin.run` exposes hosted plugins.
- **Plugin runtime** — hosted `worldos-plugin-*` subprocesses speaking
  line-delimited JSON-RPC over stdio; one `plugin:<name>` transaction per
  session; commit on clean exit, rollback on crash/timeout/protocol
  violation; sidecar `.json` manifest grants exact permissions.
- **Agent runtime** — plan → act → verify inside one transaction;
  `RulePlanner`, `LlmPlanner` (feature `llm`, OpenAI-compatible BYOK),
  `FallbackPlanner`; per-run permission profiles.
- **Interfaces** — CLI (`worldos`), JSON-RPC over stdio + WebSocket, MCP
  server, TypeScript SDK, Python SDK (stdlib-only), Tauri 2 desktop
  (builds; viewport renders analytic primitives).
- **CI** — windows-latest: fmt + clippy + tests; TS+Python SDK build;
  desktop `cargo check`.

## Verified external dependency

- **cadrum 0.8.20** (static OCCT 8.0.1, `x86_64-pc-windows-gnu` prebuilt)
  verified 2026-09-18: cube/cylinder, boolean fuse/cut/intersect,
  fillet_edges, chamfer_edges, `read_step`/`write_step` round-trip
  (vol stable at 1e-4), `read_brep`/`write_brep` round-trip,
  `Mesh::write_stl`, `Solid::mesh` tessellation, `iter_edge`/`iter_face`
  with stable `id()`s. See `docs/adr/0006-cad-engine.md`.

## Not yet working (see LIMITATIONS.md)

- Real B-rep in the project graph (no `worldos-cad` yet).
- Content-addressed artifact store.
- STEP/STL import/export through WorldOS commands.
- WorldBench, property tests, fuzz targets, crash-injection tests.
- Plugin sandboxing (native plugins are trusted local code).
- Collaboration / multi-writer.
