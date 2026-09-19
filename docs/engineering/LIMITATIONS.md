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

- **Save atomicity is journaled, not a single fs op.** The staged-save
  protocol (journal → artifact migration → wip → commit point →
  rename) recovers from interruption at every injected boundary, but
  two resources (SQLite + sidecar) cannot be atomically committed as
  one — a crash is recovered by `reconcile()` on next open, not
  prevented.
- **Migration coverage is thin.** Only schema v1 exists; no
  historical-version fixture matrix.
- **Artifact GC keeps history-reachable blobs.** Reachability walks
  live objects + relations + the undo/redo history; anything else is
  collectable. Disk-full / permission-denied mid-migration is handled
  (reneged save, source intact) but quota policy is unimplemented.

## Geometry

- **Two geometry worlds.** `geom:*` analytic primitives (component-data
  approximations) coexist with real `cad:body` B-reps — they do not
  interoperate; `geometry.measure` numbers are approximations, not
  kernel-verified.
- **Edge selection is content-hash identity.** `edge_ids` hash each
  edge's endpoints/midpoint/length — stable across BRep reloads, but
  ANY topology change invalidates them (reported as stale selection,
  never silently re-picked). No semantic selectors, no fuzzy matching.
- **`cad.measure`/`cad.regenerate` are write-schema commands** — they
  record history entries (undoable). Measuring is observationally
  read-only but occupies a transaction slot.
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
- **Desktop viewport is a software rasterizer** — real OCCT
  tessellation rendered painter-sorted on a 2D canvas; no GPU
  pipeline, no picking, no diff view, no transaction preview.

## Testing

- ~80 tests cover the golden path plus save-path failure injection and
  CAD regen cascade/rollback. Still no property tests, fuzzing,
  malformed-input campaigns, or migration matrix.
