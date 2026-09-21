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

## Agent

- **Plan-act-verify, not tool-use loop.** The agent executes a
  pre-planned command list in one transaction; it does not yet
  observe → act → inspect → re-plan iteratively.
- **No deterministic goal verifier.** Verification checks that
  top-level output `id` references still exist before committing, not
  that the user's objective is semantically true. `relation.add`
  references are checked as relations; other `id` outputs are checked
  as objects. Malformed or absent references fail the run and trigger
  rollback, retaining command outputs in the failure report. Plans that
  return a reference and delete it later in the same run also fail this
  final-state check. Commands without an `id` output have no reference
  check; this is not full postcondition or goal verification.
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
