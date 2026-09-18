# Engineering Decisions

Index of durable engineering choices. ADRs live in `docs/adr/`; this file
tracks the smaller calls and cross-cutting policies.

## ADRs

| ADR | Decision |
|-----|----------|
| 0001 | Universal Project Graph as the single semantic model |
| 0002 | Commands are the only mutation path |
| 0003 | `.worldos` = SQLite snapshot + history, atomic rewrite |
| 0004 | One `Engine` behind every interface |
| 0005 | Agents mutate only through capabilities → commands |
| 0006 | CAD engine: cadrum (static OCCT 8.0.1) behind `CadKernel` trait |

## Policies

- **Units.** Canonical CAD length unit is **millimeters** (`*_mm`
  suffixes in component data). Documented where consumed; no implicit
  unit guesses.
- **Tolerances.** Centralized in `worldos-cad` tolerance policy — no
  scattered `1e-6`/`0.001` literals in domain code.
- **Artifacts.** Content-addressed (SHA-256), sidecar directory per
  project file; graph stores digests, never blobs. Unreferenced blobs
  are gc'able garbage — artifacts are outside transaction undo scope.
- **Parametric-first.** `cad:operation` components hold regeneration
  recipes; `cad:shape` holds derived state (artifact refs, measures,
  validity). Derived state is always rebuildable.
- **Commit cadence.** One coherent change per commit; Conventional
  Commits; commit+push after every coherent update; Windows-native
  git/gh only (no WSL for repo ops).
- **Dependencies.** Prefer mature, actively maintained crates;
  prebuilt native binaries over system installs on Windows.
