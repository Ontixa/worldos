# WorldOS

**The open AI-native operating system for creating the digital and physical world.**

One world in which every tool operates. Humans, AI agents, scripts,
plugins, and external tools all work on the same governed project model
through the same command and capability systems — nothing bypasses the
kernel.

```
Intent → Requirements → Universal Project Graph
       → Design / Code / Geometry / Data / Media
       → Simulation → Verification → Reality → Observation → Iteration
```

## What exists today (Genesis milestone)

- **Universal Project Graph** — every meaningful thing is an object with
  schema-versioned components and typed relations. Files are storage,
  not semantics.
- **Governed mutations** — all changes are commands: schema-validated,
  permission-checked, grouped into atomic transactions, journaled into
  an append-only history, and undoable/redoable.
- **`.worldos` files** — a single SQLite database per project: objects,
  relations, and full history in one portable file.
- **Agent runtime** — a planner executes real commands through the
  capability layer (never raw mutation), inside one transaction,
  attributed to its own actor, then verifies its work.
- **Interfaces, one engine** — CLI, JSON-RPC (stdio + WebSocket), MCP
  server for AI tools, TypeScript/Python SDKs, and a Tauri desktop
  workspace (graph, inspector, history, 3D viewport, command palette,
  agent panel) all drive the same `Engine`.

## Quickstart

```bash
cargo build -p worldos-cli          # produces the `worldos` binary

worldos new myproject --path my.worldos
worldos command my.worldos geometry.create_primitive '{"kind":"cube","name":"box"}'
worldos agent my.worldos "create another cube next to box named housing"
worldos history my.worldos         # the agent's transaction, attributed
worldos undo my.worldos && worldos redo my.worldos
worldos validate my.worldos
```

Serve a project to SDKs: `worldos serve my.worldos --port 7799`
MCP for AI tools: `worldos mcp my.worldos`
Desktop app: `cd apps/desktop && npm run tauri dev`
Plugins: `worldos plugin list` / `worldos plugin run my.worldos stamp`
(see `examples/plugins/`)

LLM planner (BYOK, optional): set `WORLDOS_LLM_KIND=openai-compatible`,
`WORLDOS_LLM_BASE_URL`, `WORLDOS_LLM_MODEL`, `OPENAI_API_KEY` — falls back
to the rule planner automatically.

See `worldos commands my.worldos` for the live command catalog and
`worldos capabilities my.worldos` for capabilities.

## Layout

| Path | What |
|---|---|
| `crates/worldos-kernel` | UPG model, objects/components/relations, actors, requirements, deltas |
| `crates/worldos-commands` | command envelopes, schemas, transactions, history, builtin commands |
| `crates/worldos-store` | `ProjectStore` + SQLite `.worldos` persistence, migrations |
| `crates/worldos-capability` | capability registry, permission guard, `CapabilityHost`, hosted plugin runtime |
| `crates/worldos-engine` | the facade every interface drives |
| `crates/worldos-agent` | plan-act-verify agent runtime, model-provider abstraction |
| `crates/worldos-rpc` | JSON-RPC service, stdio/WS transports, MCP server |
| `crates/worldos-cli` | `worldos` binary |
| `apps/desktop` | Tauri 2 + React workspace |
| `packages/sdk-typescript` | WebSocket JSON-RPC SDK |
| `sdks/worldos-py` | stdlib-only Python SDK |
| `examples/genesis` | a real generated project |
| `examples/plugins` | hosted-plugin protocol + reference plugins |
| `docs/` + `docs/adr/` | architecture docs and decision records |

## Status

Genesis milestone implemented and tested (see `ROADMAP.md`). Forge is
underway: requirement expressions with `and`/`or`/`not` + measure terms
and `depends-on` tracing, a hosted plugin runtime (`worldos-plugin-*`
subprocesses, one attributed transaction per session), and an LLM planner
with rule fallback. The kernel stays domain-agnostic — domain lenses
arrive as commands + components, not rewrites.

License: MPL-2.0 (see `LICENSE`).
