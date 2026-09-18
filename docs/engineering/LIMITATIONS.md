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
- **No artifact store.** Large binary outputs (STEP, meshes) have no
  canonical home yet.

## Geometry

- **Analytic primitives only.** `geom:*` objects measure from component
  parameters (cube = a×b×c). There is no B-rep, no real boolean, no
  fillet, no STEP/STL I/O in the project graph yet. `geometry.measure`
  numbers are approximations, not kernel-verified.
- **Rotation ignored by measure.** `object_dims` applies scale but not
  rotation — bbox/volume are correct for volume (rotation-invariant)
  but `object_bbox` is wrong for rotated objects.

## Agent

- **Plan-act-verify, not tool-use loop.** The agent executes a
  pre-planned command list in one transaction; it does not yet
  observe → act → inspect → re-plan iteratively.
- **No deterministic goal verifier.** Verification checks that
  commands ran, not that the user's objective is semantically true.
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
