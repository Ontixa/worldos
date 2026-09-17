# ADR 0005: Agents operate through the capability layer

Status: accepted

## Context

An AI agent that mutates the graph directly would bypass validation,
permissions, undo, and attribution — breaking the product thesis.

## Decision

The agent runtime plans against live `command_schemas()` and executes via
`CapabilityHost::run_command_as(agent)` inside a transaction opened with
`begin_transaction_as`. Its work is one labeled, undoable, agent-attributed
transaction, followed by verification and an `AgentReport`. `ModelProvider`
is the seam for LLM planners; the rule-based planner is the default.

## Consequences

- An agent's actions are indistinguishable in history/permissions from a
  human's — which is the goal.
- LLM planners slot in without touching execution or governance.
- Planner quality is the bottleneck (known limitation; Forge item).
