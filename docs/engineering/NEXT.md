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

## Next (Forge slice 2: trust & proof)

5. ~~**WorldBench v0**~~ — done: `worldos-bench` crate + `bench/tasks/`
   (6 tasks) + `bench/reports/v0-baseline.json` (6/6 pass); wired into
   `scripts/bench.ps1`. Remaining: richer check kinds, corpus growth.
6. ~~**Agent tool-use loop**~~ — done: `AgentRuntime::run_spec` runs a
   bounded observe→plan→act→inspect loop in one transaction. A declared
   `done_when` predicate (requirement-expression grammar) is the only
   success exit — verified on live state each iteration; `Budget`
   (`max_iterations`/`max_commands`, per-run or per-call), `Planner::
   plan_turn` observation seam (`Observation`: prior steps, last verdict,
   remaining budget). Failures roll back with step evidence. Exposed on
   `agent.run` (`done_when`/`max_iterations`/`max_commands`) and CLI
   `--done-when`. Remaining: natural-language goals still need a caller-
   supplied predicate — RulePlanner cannot synthesize one.
7. **Adversarial hardening** — property tests (undo/redo round-trips,
   save/reopen invariants, failed-txn purity), fuzz targets
   (requirement parser, StateOp streams, JSON-RPC, plugin protocol,
   migration input), crash-injection at persistence boundaries.

## Then

8. Plugin WASM sandbox (Wasmtime) + manifest-enforced denial tests.
9. Semantic selectors for CAD topology (`top_face`,
   `faces_normal_to(+Z)`, `edges_adjacent_to(f)`) with documented
   stability guarantees.
10. Performance baselines (100 / 10k / 100k objects) + bench.ps1
    wiring.

## Explicitly deferred

- Distributed collaboration / CRDT sync.
- Plugin marketplace.
- Linux/macOS CI (supplements later, does not replace Windows).
