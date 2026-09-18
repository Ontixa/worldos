# Roadmap

## Genesis — ✅ implemented

The governed core plus narrow-but-real functionality:

- UPG kernel: objects, schema-versioned components, typed relations,
  actors, evaluable requirements, decisions
- Command system: envelopes, schemas, atomic transactions, linear
  undo/redo, attributed history
- Persistence: `.worldos` SQLite files, migrations, crash-safe writes
- Capabilities: registry, permission model, `CapabilityHost` bridge
- Engine facade shared by every surface
- Agent runtime: rule planner + model-provider abstraction, transactional
  execution, verification, actor attribution
- Surfaces: CLI, JSON-RPC (stdio + WS), MCP server, TS + Python SDKs,
  Tauri desktop workspace (object list, 3D viewport, graph, inspector,
  history, command palette, agent panel)
- Acceptance: `cargo test -p worldos-engine` (genesis.rs) + manual E2E

## Forge — in progress

Domain lenses on the same kernel. Ordered by leverage:

1. **Geometry depth** — ✅ `geometry.measure` capability (bbox, volume,
   surface area) + `volume()`/`area()`/`distance()` requirement terms over
   shared `kernel::measure`; six primitives incl. `cone`/`torus`.
   Next: parametric sketches/constraints, B-rep boundary (OCCT eval),
   mesh import/export
2. **Real agent intelligence** — ✅ `LlmPlanner` over `ModelProvider`
   (OpenAI-compatible BYOK via `WORLDOS_LLM_*`), self-repair reprompt,
   `FallbackPlanner` chain to rules, unknown-command validation, and
   per-run permission profiles (`agent.run` `permissions` input).
   Next: tool-use loop
3. **Richer requirements** — ✅ `and`/`or`/`not`/parens grammar, measure
   terms, `depends-on` tracing written by `requirement.evaluate`, and
   dependency-driven staleness (a write to a depended-on object flips the
   requirement to `stale` in the same transaction)
4. **Collaboration** — actor sessions, shared projects, op-based sync
5. **Plugin runtime** — ✅ hosted `worldos-plugin-*` executables speaking
   line-delimited JSON-RPC over stdio; one `plugin:<name>` transaction per
   session, committed on clean exit / rolled back on crash or timeout;
   discovery via `plugins/` dirs + PATH; `plugin.run` capability + MCP
   tool; sidecar `.json` manifests declaring exact permissions; Python +
   Rust reference plugins. Next: WASM sandbox, signed manifests, external
   tool adapters (Blender, KiCad…)
6. **Desktop depth** — proper GL viewport (wgpu), timeline/scrub of
   history, diff view, multi-window lenses
7. **BIM/EDA/simulation lenses** — domain component packs + validators,
   nothing special-cased in the kernel

## Deliberate non-goals for now

Full Blender/CAD/BIM/EDA replacement, distributed multi-writer
collaboration, and a plugin marketplace. Depth before breadth; the
architecture must earn each domain.
