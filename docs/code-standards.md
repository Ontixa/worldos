# Code Standards

## Rust

- Edition 2024 workspace-wide; `cargo fmt` enforced, `clippy -D warnings`
  in CI.
- Errors: `thiserror` enums per crate (`KernelError`, `CommandError`…),
  converted at boundaries; no `unwrap`/`expect` outside tests.
- Public API docs on every `pub` item in kernel/commands/capability.
- Async only where I/O demands it — the core is deliberately sync.
- New `StateOp`s must implement a correct inverse in the shared apply
  path, with an undo test.

## TypeScript / React

- Strict TS (`strict`, `noUnusedLocals`); ESM only.
- Desktop components are thin views over `api.ts` — no business logic
  in the frontend; the engine is the logic.
- No new runtime deps without justification in the PR.

## Python

- Stdlib only in `worldos.py`; type-annotated public API; py3.10+.

## Files & naming

- Snake_case Rust modules, kebab-case TS/MD files.
- Keep modules focused; split files growing past ~200 lines along
  logical seams.
- One JSON shape everywhere: snake_case on the wire, matching serde
  field names.

## Tests

- Unit tests beside the code; integration in `tests/`.
- Every fix lands with a regression test.
- Acceptance: `crates/worldos-engine/tests/genesis.rs` must stay green.

## Commits

Conventional commits; see `CONTRIBUTING.md`.
