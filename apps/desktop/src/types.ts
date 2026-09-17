export interface WsObject {
  id: string;
  type_id: string;
  name: string;
  components: Record<string, { type_id: string; version: number; data: any }>;
  tags: string[];
  meta: {
    created_at: number;
    created_by: string;
    updated_at: number;
    updated_by: string;
    revision: number;
  };
}

export interface ProjectInfo {
  id: string;
  name: string;
  path: string | null;
  object_count: number;
  relation_count: number;
  dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
  actor: string;
}

export interface CommandSchema {
  command_type: string;
  category: string;
  description: string;
  input_schema: any;
  permission: string;
  undoable: boolean;
}

export interface TxnInfo {
  id: string;
  index: number;
  actor: string;
  label: string;
  committed_at: number;
  undone: boolean;
  commands: { type: string; ok: boolean }[];
}

export interface ValidationReport {
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
  status: string;
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
