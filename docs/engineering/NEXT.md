# Next

Prioritized, concrete engineering work. Kept short on purpose — this is
the next ~5 items, not the backlog.

## Now (Forge slice 1: real CAD)

1. **`worldos-artifact`** — content-addressed store (SHA-256, sidecar
   `<project>.artifacts/` dir, put/get/exists/verify/gc + corruption
   tests). Foundation for CAD outputs.
2. **`worldos-cad` + `worldos-adapter-cadrum`** — `CadKernel` trait over
   cadrum (OCCT 8.0.1). `cad:operation` recipes + `cad:shape` derived
   state. Commands: `cad.create_{box,cylinder,sphere}`,
   `cad.boolean.{union,subtract,intersect}`, `cad.fillet`,
   `cad.chamfer`, `cad.measure`, `cad.transform`,
   `cad.import_step`, `cad.export_{step,stl}`. Canonical unit = mm.
3. **Parametric regeneration** — `cad.set_param` → rebuild → update
   `cad:shape` → existing staleness machinery propagates.
4. **Vertical-slice proof** — create → measure → STEP export →
   reimport → save → reopen → regenerate → same verified result.

## Next (Forge slice 2: trust & proof)

5. **WorldBench v0** — YAML task format, deterministic runner,
   non-LLM baseline, initial corpus (graph/transactions/persistence/
   cad). Evidence-rich run records.
6. **Agent tool-use loop** — bounded observe→act→inspect→replan with
   iteration caps and deterministic goal verification (separate from
   "commands ran").
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
