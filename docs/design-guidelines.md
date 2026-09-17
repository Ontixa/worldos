# Design Guidelines

## The product thesis, restated as design rules

1. **One world.** If a feature would create a parallel model, redesign it
   to be components + relations on the UPG.
2. **Governance is the feature.** Speed that bypasses commands is a bug,
   not an optimization.
3. **Objects compose, types don't.** Prefer a new component type on an
   existing object over a new object type that duplicates structure.
4. **Lenses, not silos.** A "CAD view" is a projection of geometry +
   requirement + cost components — not a separate document.

## Adding domain functionality

- New data → new component `type_id` + versioned schema (in
  `kernel/known.rs` if builtin, or a domain crate otherwise).
- New mutations → new commands with honest `input_schema` and a real
  inverse.
- New read-only analytics → capabilities (declared permissions,
  determinism class).
- New domain rules → `Validator`s producing diagnostics, never silent
  mutation.

## UX principles (desktop)

- Dense, keyboard-first, dark IDE aesthetic — this is an OS workspace,
  not a consumer app.
- Every control that changes state is a command (palette entry,
  shortcut, or panel action) — visible in History.
- Selection is the shared cursor: object list ↔ 3D ↔ graph ↔ inspector
  all track one `selectedId`.
- The agent panel shows *what commands ran*, not just a chat bubble.

## Performance stance

In-memory graph, snapshot persistence — correct for Genesis scale
(thousands of objects). When profiling shows strain: incremental save,
spatial indexes, and LOD in the viewport — behind existing interfaces.
