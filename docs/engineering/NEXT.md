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

5. ~~**WorldBench**~~ — done: `worldos-bench` crate + `bench/tasks/`
   (13 tasks) + reports under `bench/reports/` (13/13 pass); wired into
   `scripts/bench.ps1`. Richer check kinds shipped: `history` journal
   invariants, `snapshot` state digests (exact undo/redo, save/reopen,
   failed-command purity), `object_count`/`relation_count`,
   `no_dangling_relations`, `field_absent`, `valid`, `artifact_verified`
   (sidecar re-hash), `file_digest`; plus `capability.run` steps
   (`agent.run`, `project.inspect`, `geometry.measure`) and `${bench.dir}`
   tempdir interpolation. Corpus covers relation graphs + cascade undo,
   requirement staleness, mesh export/import round-trips, agent
   transaction rollback, history across reopen, failure purity, and CAD
   artifact undo/save_as migration.
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
7. **Adversarial hardening** — partly done: property tests shipped
   (`proptest`): engine undo/redo round-trips + failed-transaction
   purity (`worldos-engine/tests/property_state_machine.rs`),
   save/reopen invariants over random projects incl. journal
   (`property_persistence.rs`), store-level snapshot round-trips +
   crash injection (`worldos-store/tests/crash_recovery.rs` — real
   `abort()` mid-write and post-commit, truncation/garbage rejection).
   Found + fixed: stale journal index after redo-tail truncation,
   `project.rename` undo writing `settings["__name"]`, unbounded
   `core:contains` cycle traversal. Remaining: fuzz targets
   (requirement parser, StateOp streams, JSON-RPC, plugin protocol,
   migration input) and in-process I/O-fault injection inside the
   store (current crash coverage is process abort, not injected
   `Write` errors).

## Then

8. Plugin WASM sandbox (Wasmtime) + manifest-enforced denial tests.
9. ~~Semantic selectors for CAD topology~~ — done: `worldos-cad`
   `Selector` expression language (`top_face`, `faces_normal_to`,
   `faces_axis_to`, `faces_of_kind`, `face_extreme`, `edges_adjacent_to`,
   `edges_extreme`, set ops) resolved over `CadKernel::topology_view`;
   `edge_select` on `cad.fillet`/`cad.chamfer`, `select` on
   `cad.measure`, `cad.select` probe command; recipes persist the
   expression and re-resolve on replay. Contract + honest limits in
   `docs/cad-selectors.md`.
10. ~~Performance baselines~~ — done: `worldos-bench perf` measures
    create/save/reopen/find/search/undo at 100 / 10k / 100k objects
    through the real command path; `scripts/bench.ps1 -Suite perf`
    (default `100,10000`, 100k opt-in) writes
    `bench/reports/worldbench-perf-*.json`. Remaining: none for the
    baseline itself — statistical rigor (multi-run, percentiles) is
    deferred with the fuzz targets.

## Explicitly deferred

- Distributed collaboration / CRDT sync.
- Plugin marketplace.
- Linux/macOS CI (supplements later, does not replace Windows).
