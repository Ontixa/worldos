# WorldOS Architecture

## The one rule

**Every mutation is a command. Every command is a transaction member.
Every transaction is history.**

No interface — GUI, CLI, MCP, SDK, agent, or future plugin — mutates the
project graph directly. They all funnel into `Engine`, which routes
through validation → permissions → command handler → `StateOp` deltas →
transaction journal.

```
┌─────────┬─────────┬─────────┬─────────┬──────────┐
│ Desktop │   CLI   │   MCP   │   SDK   │  Agent   │
└────┬────┴────┬────┴────┬────┴────┬────┴────┬─────┘
     └─────────┴────┬────┴─────────┴─────────┘
                    ▼
        ┌───────────────────────┐
        │        Engine         │  facade + orchestration
        └───────────┬───────────┘
        ┌───────────┴───────────┐
        │   Capability runtime  │  governed ops behind descriptors
        └───────────┬───────────┘
        ┌───────────┴───────────┐
        │   Command registry    │  schema-validated handlers
        │   Transactions        │  atomic units of StateOps
        │   History             │  undo/redo journal, attribution
        └───────────┬───────────┘
        ┌───────────┴───────────┐
        │   Kernel (UPG)        │  objects, components, relations,
        │                       │  actors, requirements, validation
        └───────────┬───────────┘
        ┌───────────┴───────────┐
        │   Store               │  .worldos = SQLite, one file,
        │                       │  snapshot + history, migrations
        └───────────────────────┘
```

## Universal Project Graph

- **Object** — id (ULID), `type_id`, name, tags, metadata (created_by,
  revision…), and a bag of **components**.
- **Component** — `{type_id, version, data: JSON}`. Schema-versioned so
  domains can evolve without migrations that understand semantics.
- **Relation** — typed directed edge (`core:contains`, `core:satisfies`,
  `core:depends-on`…) with optional metadata.
- **Actor** — human/agent/service/plugin/system identity carrying
  permissions; every record is attributed.

An object is *not* "a CAD part" or "a file" — it's a node that can carry
`geom:geometry` + `core:requirement-status` + `code:file` simultaneously.
Domain lenses read the same graph.

## Commands & transactions

- `CommandEnvelope` — serializable intent `{command_type, actor, inputs}`.
- `CommandSchema` — self-describing input/output JSON-schema; the
  palette, MCP tools, and agent planners all enumerate it live.
- Handlers emit `StateOp`s — invertible deltas (insert/update/delete of
  objects, components, relations). Forward and inverse share one code
  path, which is what makes undo exact.
- A transaction is an atomic, labeled, actor-attributed group of
  commands. Failure rolls the whole thing back.
- `History` is append-only with a linear-undo cursor: undo moves the
  cursor back, redo replays forward; new writes truncate the redo tail.

## Capabilities

A `Capability` is a described operation (inspect, search, measure,
agent.run…) with declared permissions, determinism class, and provider.
Capabilities mutate only through `CapabilityHost::run_command*` — so an
agent's actions are indistinguishable in history from a human's.

## Agents

`worldos-agent` = plan → act → verify, on top of `CapabilityHost`:

1. **Plan** — `RulePlanner` (offline, deterministic) or `LlmPlanner`
   (feature `llm`, BYOK OpenAI-compatible endpoints via `WORLDOS_LLM_*`),
   wrapped in `FallbackPlanner` so provider failure degrades to rules.
   Malformed model output gets one self-repair reprompt; proposed
   commands are validated against live `command_schemas()`.
2. **Act** — `begin_transaction_as(agent)` → `run_command_as` per step.
3. **Verify** — resolve created objects, check they exist, emit a
   structured `AgentReport` (also recorded as a `core:agent-task` object).

## Plugins

`worldos-plugin-*` executables are hosted subprocesses: the engine serves
a line-delimited JSON-RPC channel over the plugin's stdin/stdout, scoped
to a read surface + `command.execute`. The session runs as the
`plugin:<name>` actor inside ONE transaction — committed on clean exit,
rolled back on crash/timeout/protocol violation. Discovery scans
`plugins/` dirs, `~/.worldos/plugins`, `WORLDOS_PLUGIN_PATH`, and PATH;
script extensions get interpreters (`.py`, `.ps1`, `.cmd`). A sidecar
`<stem>.json` manifest can declare `permissions` — when present the
plugin actor gets exactly those grants; otherwise it inherits the
agent-style default. The same channel is exposed as the `plugin.run`
capability so agents/MCP/SDK clients can invoke plugins.

## Persistence

`.worldos` = SQLite (WAL). Whole-graph snapshot tables + history tables
written transactionally; `PRAGMA user_version` drives migrations. The
in-memory `Project` is the working set; save = atomic rewrite.

## Interfaces

- **CLI** — scriptable shell of the engine.
- **JSON-RPC** — `rpc` (stdio NDJSON) and `serve` (WebSocket).
- **MCP** — `mcp` exposes engine ops as tools for AI clients.
- **SDKs** — thin clients over the transports; no logic duplication.
- **Desktop** — Tauri embeds `Engine` in-process; identical semantics.

## What this buys

A plugin written tomorrow in any language, driving `worldos rpc`, gets
permissions, validation, undo, audit history, and interop with every
other tool — for free. That is the "operating system" claim.
