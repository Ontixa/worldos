# Project Overview / PDR

## Product

WorldOS — the open AI-native operating system for creating the digital
and physical world. One governed project model; every tool, human, and
agent operates inside it.

## Problem

Creation tooling is fragmented: CAD, code editors, requirements trackers,
BIM/EDA suites, simulators, and AI assistants each own a private model
of the world. Data is exported, translated, and lost between them; AI
agents bolted on top can't see or safely change the real state.

## Approach

A shared semantic substrate — the **Universal Project Graph** — where
objects carry composable, schema-versioned components and typed
relations, and *every* mutation flows through validated, permissioned,
journaled commands. Domain lenses (CAD, code, BIM…) are views and
command packs over the same graph, not separate apps.

## Requirements (Genesis)

| # | Requirement | Status |
|---|---|---|
| R1 | Object/component/relation kernel, domain-agnostic | ✅ |
| R2 | Command layer: schemas, validation, permissions | ✅ |
| R3 | Transactions: atomic, labeled, attributed | ✅ |
| R4 | History: persistent journal, exact undo/redo | ✅ |
| R5 | `.worldos` persistence (SQLite, migratable) | ✅ |
| R6 | Capability runtime + registry | ✅ |
| R7 | Agent runtime on the governed path | ✅ |
| R8 | CLI + RPC + MCP + SDKs | ✅ |
| R9 | Desktop workspace (graph, inspector, 3D, agent) | ✅ |
| R10 | Docs + tests + CI | this PR |

## Non-goals (Genesis)

Full domain depth (CAD/BIM/EDA/sim), multi-user collaboration, plugin
marketplace. See `ROADMAP.md`.

## Success metric

A second tool — an SDK script, an MCP client, an agent — can observe and
mutate the same project the GUI user sees, with identical governance.
Demonstrated: `examples/genesis` + `crates/worldos-engine/tests/genesis.rs`.
