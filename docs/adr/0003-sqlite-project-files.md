# ADR 0003: `.worldos` is a single SQLite file

Status: accepted

## Context

Projects need durable, portable, crash-safe storage that holds both graph
snapshot and full history — without running a server.

## Decision

One `.worldos` file = one SQLite database (WAL): `project`, `objects`,
`components`, `relations`, `transactions`, `commands`, `ops` tables;
`user_version` drives migrations. Save is one atomic transaction that
replaces the snapshot and appends new history.

## Consequences

- Atomicity/durability for free; one file to copy/share.
- History is queryable with plain SQL by external tools.
- Whole-snapshot saves bound project scale for now — acceptable at
  Genesis scale; incremental persistence is a Forge item behind the
  same `ProjectStore` trait.
- Files must live on real filesystems (SQLite locking breaks on 9p/WSL
  mounts and some network drives).
