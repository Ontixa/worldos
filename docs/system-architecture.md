# System Architecture

> Narrative version in `../ARCHITECTURE.md`; this file is the
> reference-level view.

## Crates and dependencies

```
worldos-kernel      (no internal deps) — model, ids, actors, validation
worldos-commands    → kernel           — envelopes, schemas, txn, history
worldos-store       → kernel, commands — snapshot + SQLite persistence
worldos-capability  → kernel, commands — registry, descriptors, host trait
worldos-engine      → all above        — facade: session, search, events
worldos-agent       → capability       — planner, runtime, providers
worldos-rpc         → engine, agent    — JSON-RPC, stdio/WS, MCP server
worldos-cli         → engine, rpc, agent — the `worldos` binary
worldos-desktop     → engine, agent    — Tauri app (own workspace)
```

Dependency direction is strict: outer layers never reach inward past
`Engine`, and `kernel` never knows who is calling.

## Data model

- `Object { id: Ulid, type_id, name, components: Map<String, Component>, tags, meta }`
- `Component { type_id, version, data: serde_json::Value }`
- `Relation { id, type_id, from, to, meta }`
- `Actor { id, kind: Human|Agent|Service|Plugin|System, permissions }`

## Mutation flow

```
envelope → schema check → permission check → handler → Vec<StateOp>
        → apply ops to Project → record CommandRecord
        → commit TransactionRecord to History → emit EngineEvent
```

`StateOp` variants (insert/update/remove × object/component/relation)
are invertible; `History::undo` applies inverses in reverse order.

## Persistence

`SqliteStore` tables: `project`, `objects`, `components`, `relations`,
`transactions`, `commands`, `ops`, plus `user_version` for migrations.
Save = single SQLite transaction replacing snapshot rows + appending
history rows. Load = reconstruct graph + replay cursor position.

## Surfaces

| Surface | Transport | Entry |
|---|---|---|
| CLI | in-process | `worldos <cmd>` |
| JSON-RPC stdio | NDJSON pipes | `worldos rpc file` |
| JSON-RPC WS | tungstenite | `worldos serve file` |
| MCP | stdio, protocol 2025-06-18 | `worldos mcp file` |
| Desktop | Tauri invoke | `apps/desktop` |
| TS SDK | WS client | `@worldos/sdk` |
| Py SDK | subprocess stdio | `worldos` module |

All produce identical command/history semantics.
