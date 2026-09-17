# ADR 0004: One Engine facade behind every interface

Status: accepted

## Context

CLI, GUI, RPC, MCP, SDKs and agents each tempt their own logic — which is
exactly how semantics drift and bypasses appear.

## Decision

`worldos-engine::Engine` is the only orchestration point. Interfaces are
thin transports: `worldos-rpc` dispatches JSON onto `Engine`, the CLI
formats its output, the desktop holds an `Engine` in Tauri state, MCP
maps tools to engine methods. No interface implements project logic.

## Consequences

- Behavioral parity across surfaces by construction.
- New capability/command lands once and appears everywhere (MCP tool
  list, palette, SDK).
- Engine must stay embeddable (no transport assumptions inside it).
