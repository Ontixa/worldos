# Contributing

## Setup

- Rust (MSVC on Windows): `cargo build --workspace`
- Node ≥ 20 for the TS SDK and desktop frontend: `npm install`
- Python ≥ 3.10 for the Python SDK (no deps)

## Verify before you push

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck --workspaces --if-present
python -m py_compile sdks/worldos-py/worldos.py
```

Desktop: `cd apps/desktop && npm run build && cd src-tauri && cargo check`

## The invariants

These are architectural commitments — PRs that violate them will be
reworked, not merged:

1. **Commands are the only mutation path.** If new functionality writes
   project state outside a command handler, it's a bug.
2. **No interface-specific logic.** Desktop, CLI, MCP, SDKs and agents
   share `Engine`. Fix behavior in the engine, not the client.
3. **Components are schema-versioned.** Add a version bump + tolerant
   reader when changing component shapes.
4. **Agents use capabilities.** An agent path that bypasses
   `CapabilityHost::run_command*` is a security hole.
5. **Undo must be exact.** New `StateOp`s need a correct inverse — the
   delta apply path is shared, so test undo alongside the feature.

## Adding a command

1. Write the handler in `crates/worldos-commands/src/builtin/`.
2. Give it a `CommandSchema` (input JSON-schema, permission, category).
3. Register it in `builtin/mod.rs`.
4. It is now automatically available to CLI, RPC, MCP tools, SDKs, the
   desktop palette, and agent planners — that's the point.

## Commit style

Conventional commits (`feat:`, `fix:`, `refactor:`…), scoped where
helpful (`feat(agent): …`). Keep diffs focused.
