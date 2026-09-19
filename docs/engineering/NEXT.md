# Next

Prioritized, concrete engineering work. Kept short on purpose — this is
the next ~5 items, not the backlog.

## Done (Forge slice 1: real CAD) — shipped on main

1. ~~`worldos-artifact`~~ — content-addressed store shipped.
2. ~~`worldos-cad` + `worldos-adapter-cadrum`~~ — all commands live:
   `cad.create_{box,cylinder,sphere}`, `cad.boolean`, `cad.fillet`,
   `cad.chamfer`, `cad.transform`, `cad.measure`,
   `cad.import_step`, `cad.export_{step,stl}`.
3. ~~Parametric regeneration~~ — `cad.set_param`/`cad.regenerate`
   replay `cad:operation` recipes; `core:derived-from` edges mark
   direct dependents `cad:shape.stale`.
4. ~~Vertical-slice proof~~ — `worldos-engine/tests/cad_vertical.rs`
   (5 tests: create→measure→STEP→reimport→undo/redo→save→reopen→
   regenerate; feature chain; save_as artifact migration; failure
   paths).

## Done (Forge slice 2: CAD workbench campaign) — on build/cad-workbench-p06

5. ~~**Durable staged save**~~ — journal + artifact migration + wip
   verify + commit point + rename; `reconcile()` recovery; overwrite
   grant; `Engine::open` never creates files.
6. ~~**Multi-level parametric regen**~~ — transitive staleness,
   topo-ordered `cascade`/`all_stale`, cycle + stale-input rejection,
   atomic rollback, content-hash edge ids.
7. ~~**WorldBench v1**~~ — 17-task corpus incl. failure injection;
   format_version 2 report with provenance metadata.
8. ~~**CAD workbench UI slice**~~ — desktop renders real OCCT
   tessellation; inspector edits params, shows stale/deps; CLI demo
   `examples/cad-bracket` via `worldos batch` command plan.

## Next (trust & proof, cont.)

9. **Agent tool-use loop** — bounded observe→act→inspect→replan with
   iteration caps and deterministic goal verification (separate from
   "commands ran").
10. **Adversarial hardening** — property tests (undo/redo round-trips,
    save/reopen invariants, failed-txn purity), fuzz targets
    (requirement parser, StateOp streams, JSON-RPC, plugin protocol,
    migration input).

## Then

11. Plugin WASM sandbox (Wasmtime) + manifest-enforced denial tests.
12. Semantic selectors for CAD topology (`top_face`,
    `faces_normal_to(+Z)`, `edges_adjacent_to(f)`) with documented
    stability guarantees — beyond today's content-hash edge ids.
13. Read-schema commands (`cad.measure` without a history entry).
14. Performance baselines (100 / 10k / 100k objects) + bench.ps1
    wiring.

## Explicitly deferred

- Distributed collaboration / CRDT sync.
- Plugin marketplace.
- Linux/macOS CI (supplements later, does not replace Windows).
