/**
 * WorldOS SDK types — mirror the kernel/command model over the wire.
 * IDs are ULID strings; components are schema-versioned JSON.
 */

export type ObjectId = string;
export type TransactionId = string;
export type CommandId = string;

export interface WsObject {
  id: ObjectId;
  type_id: string;
  name: string;
  components: Record<string, { type_id: string; version: number; data: unknown }>;
  tags: string[];
  meta: {
    created_at: number;
    created_by: string;
    updated_at: number;
    updated_by: string;
    revision: number;
  };
}

export interface CommandReceipt {
  command_id: CommandId;
  transaction_id: TransactionId;
  output: unknown;
}

export interface CommandSchema {
  command_type: string;
  category: string;
  description: string;
  input_schema: unknown;
  output_schema?: unknown;
  permission: string;
  undoable: boolean;
}

export interface CapabilityDescriptor {
  id: string;
  version: string;
  description: string;
  provider: { id: string; name: string };
  input_schema: unknown;
  permissions: string[];
  determinism: string;
  execution: string;
}

export interface TransactionInfo {
  id: TransactionId;
  index: number;
  actor: string;
  label: string;
  committed_at: number;
  undone: boolean;
  commands: { type: string; ok: boolean; id: string }[];
  ops: string[];
}

export interface ValidationReport {
  validator_runs: { validator_id: string; diagnostics: number; duration_ms: number }[];
  diagnostics: {
    severity: "info" | "warning" | "error";
    code: string;
    message: string;
    object_id?: string;
    hint?: string;
  }[];
  passed: boolean;
}

export interface AgentReport {
  run_id: string;
  agent: string;
  goal: string;
  status: "planned" | "succeeded" | "failed" | "unsupported";
  steps: { index: number; command: string; note: string; ok: boolean; error?: string }[];
  transaction_id?: string;
  created_objects: string[];
  verification: string[];
  summary: string;
}

export interface GraphView {
  nodes: { id: string; name: string; type: string; components: string[] }[];
  edges: { id: string; type: string; from: string; to: string }[];
}
