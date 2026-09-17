/** Thin typed wrapper over the Tauri command bridge to the Engine. */
import { invoke } from "@tauri-apps/api/core";
import type {
  AgentReport,
  CommandSchema,
  GraphView,
  ProjectInfo,
  TxnInfo,
  ValidationReport,
  WsObject,
} from "./types";

export const api = {
  projectNew: (name: string, path: string) =>
    invoke<ProjectInfo>("project_new", { name, path }),
  projectOpen: (path: string) => invoke<ProjectInfo>("project_open", { path }),
  projectSave: (path?: string) =>
    invoke<ProjectInfo>("project_save", { path: path ?? null }),
  projectInfo: () => invoke<ProjectInfo | null>("project_info"),

  objectList: (typeId?: string) =>
    invoke<WsObject[]>("object_list", { typeId: typeId ?? null }),
  objectGet: (idOrName: string) =>
    invoke<WsObject>("object_get", { idOrName }),
  objectRelations: (id: string) =>
    invoke<{ id: string; type: string; from: string; to: string }[]>(
      "object_relations",
      { id },
    ),
  graph: () => invoke<GraphView>("graph"),
  search: (query: Record<string, unknown>) =>
    invoke<{ id: string; name: string; type: string }[]>("search", { query }),

  commandExecute: (commandType: string, input: unknown) =>
    invoke<{ command_id: string; transaction_id: string; output: unknown }>(
      "command_execute",
      { commandType, input },
    ),
  commandList: () => invoke<CommandSchema[]>("command_list"),
  capabilityList: () => invoke<unknown[]>("capability_list"),

  history: (limit = 100) => invoke<TxnInfo[]>("history", { limit }),
  undo: () => invoke<{ undone: string | null }>("undo"),
  redo: () => invoke<{ redone: string | null }>("redo"),
  validate: () => invoke<ValidationReport>("validate"),
  agentRun: (goal: string) => invoke<AgentReport>("agent_run", { goal }),
};
