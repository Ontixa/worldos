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

## Forge — next

Domain lenses on the same kernel. Ordered by leverage:

1. **Geometry depth** — parametric sketches/constraints, B-rep boundary
   (OCCT eval), mesh import/export, measurement queries as capabilities
2. **Real agent intelligence** — LLM planner over `ModelProvider`
   (OpenAI/Anthropic/local), tool-use loop, planner self-repair,
   multi-agent actors with distinct permissions
3. **Richer requirements** — expression language over component fields
   (`housing.volume >= x`), dependency tracing via relations
4. **Collaboration** — actor sessions, shared projects, op-based sync
5. **Plugin runtime** — WASM/hosted capabilities with declared
   permissions; external tool adapters (Blender, KiCad…) speaking RPC
6. **Desktop depth** — proper GL viewport (wgpu), timeline/scrub of
   history, diff view, multi-window lenses
7. **BIM/EDA/simulation lenses** — domain component packs + validators,
   nothing special-cased in the kernel

## Deliberate non-goals for now

Full Blender/CAD/BIM/EDA replacement, distributed multi-writer
collaboration, and a plugin marketplace. Depth before breadth; the
architecture must earn each domain.
