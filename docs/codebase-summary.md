# Codebase Summary

## crates/worldos-kernel

The domain-free semantic core.

- `ids.rs` — ULID newtypes (`ObjectId`, `CommandId`, `ActorId`…), `now_ms`
- `model.rs` — `Object`, `Component`, `Relation`, `TypeId`
- `actor.rs` — `Actor`, `ActorKind`, permission set
- `delta.rs` — `StateOp` + `apply(project, ops, inverse)` — the single
  mutation path shared by commands, undo, and replay
- `project.rs` — `Project` graph + name/type indexes
- `known.rs` — builtin type/component/relation constants
- `schema.rs` — minimal JSON-schema validator for component/command input
- `requirement.rs` — requirement expression evaluation: `and`/`or`/`not`/
  parens grammar, `exists*`/`count`/`object()`/`volume`/`area`/`distance`
  terms, dependency tracing (returns referenced objects)
- `measure.rs` — analytic geometry measures (dims, bbox, volume, area,
  distance) shared by the `geometry.measure` capability and requirement
  terms
- `validation.rs` — `Validator` trait, `ValidationReport`, diagnostics
- `search.rs` — `SearchQuery` (text/type/tag/component)
- `events.rs` — `EngineEvent` (object/txn/project signals)

## crates/worldos-commands

- `envelope.rs` — `CommandEnvelope` (intent), `CommandReceipt`, `CommandRecord`
- `schema.rs` — `CommandSchema` (input/output JSON-schema, permission)
- `handler.rs` — `CommandHandler` trait, `CommandContext`
- `txn.rs` — `Transaction`, `TransactionRecord`
- `history.rs` — `History` journal, linear-undo cursor
- `builtin/` — object, relation, document, code, geometry, meta,
  requirement command handlers

## crates/worldos-store

- `store.rs` — `ProjectStore` trait + `MemoryStore`
- `snapshot.rs` — serializable whole-graph snapshot
- `sqlite.rs` — `SqliteStore`: WAL, atomic writes, `user_version`
  migrations, history persistence
- `tests/roundtrip.rs` — save/reopen, missing file, interrupted save

## crates/worldos-capability

- `descriptor.rs` — `CapabilityDescriptor` (permissions, determinism…)
- `registry.rs` — registration + permission-checked dispatch
- `host.rs` — `CapabilityHost` bridge (run commands, txn control,
  schemas, validation, object resolution, project path)
- `builtin.rs` — inspect/search/validate/export/measure capabilities
- `plugin.rs` — hosted plugin runtime: `worldos-plugin-*` subprocesses
  speaking line-delimited JSON-RPC over stdio; `plugin.run` capability,
  discovery, interpreter dispatch, timeout, commit/rollback

## crates/worldos-engine

- `engine.rs` — `Engine` facade: lifecycle, execute(+as actor),
  transactions, undo/redo, capabilities, validation, search, events
- `diff.rs` — project-vs-project diff
- `tests/genesis.rs` — the acceptance test

## crates/worldos-agent

- `planner/` — `Planner` trait + `PlannedStep`; `rules.rs` (deterministic
  offline planner), `llm.rs` (`LlmPlanner` over `ModelProvider` with
  self-repair reprompt + `FallbackPlanner` chain)
- `runtime.rs` — plan→act→verify loop in one transaction
- `capability.rs` — `AgentRun` capability (`agent.run`)
- `provider.rs` — `ModelProvider` trait, `EchoProvider`,
  `OpenAiCompatible` (feature `llm`, env-configured BYOK)
- `report.rs` — `AgentReport`, step records

## crates/worldos-rpc

- `proto.rs` — JSON-RPC 2.0 types
- `service.rs` — `RpcService::handle` — method dispatch onto `Engine`
- `stdio.rs` — NDJSON stdio transport (`worldos rpc`)
- `ws.rs` — per-connection thread WebSocket server (`worldos serve`)
- `mcp.rs` — MCP server (initialize/tools/list/tools/call)

## crates/worldos-cli

`main.rs` (clap defs) + `cmd.rs` (dispatch) + `out.rs` (json/human output).

## apps/desktop

Tauri 2: `src-tauri` embeds `Engine` via `Mutex<Option<Engine>>` state;
React frontend (`src/components/*`): ObjectList, Viewport3D (software
renderer), GraphView (SVG), Inspector (component editing via commands),
HistoryPanel, AgentPanel, CommandPalette (live schemas).

## SDKs

- `packages/sdk-typescript` — `WorldosClient` over WebSocket
- `sdks/worldos-py` — `worldos` module over `worldos rpc` subprocess
