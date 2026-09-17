# ADR 0002: Commands + invertible StateOps are the only mutation path

Status: accepted

## Context

Undo, audit, permissions, collaboration, and safe agents all require
governed change. Ad-hoc mutation APIs inevitably bypass governance.

## Decision

All mutations are `CommandEnvelope`s → schema-validated handlers →
`Vec<StateOp>` applied inside a `Transaction` recorded in `History`.
`StateOp`s are invertible; undo = inverse application, redo = replay.
There is no sanctioned "raw write" anywhere in the stack — including for
agents and plugins.

## Consequences

- Undo/redo, attribution, and audit are free for every feature forever.
- Handlers must compute minimal deltas; slightly more work per command.
- `StateOp` coverage must grow with the model; each new op needs an
  inverse and a round-trip undo test.
