# ADR 0006: CAD engine — native OCCT via cadrum

Status: accepted

## Context

WorldOS needs a real B-rep kernel: STEP/STL I/O, booleans, fillets,
chamfers, tessellation, topology traversal, and measurements that are
kernel-verified rather than analytic approximations. The kernel must be
embeddable (no server process), work on `x86_64-pc-windows-gnu` CI, and
not require contributors to build OCCT from source.

Two candidates were evaluated empirically on 2026-09-18 (scratch
project `D:\temp\occt-verify`, not committed):

**occt-wasm 3.3.0** — OCCT 8.0.0 compiled to WASM, hosted on wasmtime.

- `OcctKernel::new()` fails as published: the WASM module declares ~18
  emscripten/WASI `env::*` imports and the crate instantiates it with an
  empty `Linker`. We supplied the imports manually to evaluate further.
- With imports stubbed and `_initialize` called: box/cut/volume/
  validity/tessellation work; `to_brep`/`from_brep` round-trips exactly.
- STEP I/O is dead in this build — it routes through `fopen("/tmp/…")`
  on Emscripten MEMFS, and the module imports no `open`/`mkdir`/`fd`
  syscalls, so the path can never be created.
- API gaps: `get_sub_shapes` returns raw `u32` ids while `fillet`
  requires private `ShapeHandle` values; `export_brep_binary` traps with
  an out-of-bounds memory access.
- Rejected: the published host path cannot instantiate, and the build
  cannot do STEP — the format that matters most for CAD interchange.

**cadrum 0.8.20** — cxx bindings over statically linked OCCT 8.0.1,
with prebuilt binaries downloaded at build time for
`x86_64-pc-windows-msvc` and `-gnu`.

- Built and linked on `x86_64-pc-windows-gnu` with zero system deps.
- Smoke test (118 ms): cube vol=100000.0, area=16000.0; boolean
  cut/fuse/intersect volumes all correct; `fillet_edges` (15 edges) and
  `chamfer_edges` produce real features; `write_step`→`read_step`
  round-trip preserves volume exactly (98429.2037 mm³); `write_brep`/
  `read_brep` round-trips; STL export and `Solid::mesh` tessellation
  work; `iter_edge`/`iter_face`/`id()` give stable topology ids usable
  as semantic selectors.
- Actively maintained (45 releases in ~5 months), MIT/Apache-2.0.

## Decision

Adopt **cadrum** as the OCCT backend, behind a WorldOS-owned
`CadKernel` trait in a new `worldos-cad` crate (with the cadrum
implementation in `worldos-adapter-cadrum`). The trait keeps the
kernel replaceable and lets tests use a fakes/metrics shim; cadrum is
the production adapter.

Canonical CAD unit: **millimeters** (`*_mm` in component data).
Kernel outputs (BRep/STEP/STL bytes) are persisted through the
content-addressed artifact store, never inline in components — the
graph stores digests and derived measures only.

## Consequences

- Real B-rep modeling, STEP/STL I/O, and topology traversal are
  available at native speed on Windows (MSVC + gnu) today.
- OCCT ships prebuilt via crates.io — no CMake/OCCT install for
  contributors; CI stays `windows-latest` stock.
- ~120 MB static link per consumer binary; desktop/CLI binaries grow.
- The `CadKernel` trait boundary means `occt-wasm` (or a future WASM
  plugin-kernel) can be revisited if its host/STEP story matures —
  the decision is recorded, not welded.
- Shape handles/edge ids are kernel-internal; semantic selectors
  (`faces_normal_to`, `edges_adjacent_to`, …) must be re-resolved
  after every regeneration — topology ids are not a stable API.
