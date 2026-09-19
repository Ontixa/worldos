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

/** Real BRep tessellation of a cad:body, mm (worldos-cad MeshData). */
export interface CadMesh {
  id: string;
  brep: string;
  stale: boolean;
  generator: string | null;
  units: "mm";
  mesh: {
    positions: [number, number, number][];
    normals: [number, number, number][];
    indices: number[];
    face_ids: number[];
  };
  measures: {
    volume_mm3: number;
    area_mm2: number;
    bbox: { min_mm: [number, number, number]; max_mm: [number, number, number] };
    center_mm: [number, number, number];
  };
  topology: {
    solids: number; faces: number; edges: number;
    is_solid: boolean; is_valid: boolean;
    edge_ids: number[]; face_ids: number[];
    edges_detail?: { id: number; length_mm: number; start_mm: number[]; end_mm: number[]; mid_mm: number[] }[];
  };
}
