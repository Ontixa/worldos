# ADR 0001: The Universal Project Graph is the primary model

Status: accepted

## Context

Creation tools center on files (CAD documents, code trees, spreadsheets).
Files serialize well but make cross-domain semantics, undo, attribution,
and agent access second-class.

## Decision

The semantic center is an in-memory graph: `Object`s carrying
schema-versioned `Component`s, connected by typed `Relation`s, all under
`Actor` attribution. Files (`.worldos`) are a storage/transport format —
a projection, not the model.

## Consequences

- Anything can be an object; objects span domains by composition.
- Tools interoperate on semantics, not file formats.
- Cost: we own indexing, traversal, and versioning infrastructure that
  filesystems used to provide.
