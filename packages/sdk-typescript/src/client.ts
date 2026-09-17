/**
 * WorldosClient — JSON-RPC 2.0 over WebSocket against `worldos serve`.
 *
 * Every call maps to the same engine methods the CLI and MCP use, so a
 * script sees and mutates exactly what the desktop app sees.
 */

import type {
  AgentReport,
  CapabilityDescriptor,
  CommandReceipt,
  CommandSchema,
  GraphView,
  TransactionInfo,
  ValidationReport,
  WsObject,
} from "./types.js";

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class RpcError extends Error {
  constructor(
    public readonly code: number,
    message: string,
    public readonly data?: unknown,
  ) {
    super(message);
    this.name = "RpcError";
  }
}

export class WorldosClient {
  private ws: WebSocket;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private timeoutMs: number;

  private constructor(ws: WebSocket, timeoutMs = 30_000) {
    this.ws = ws;
    this.timeoutMs = timeoutMs;
    ws.onmessage = (ev: MessageEvent) => this.onMessage(ev);
    ws.onclose = () => this.failAll(new RpcError(-32000, "connection closed"));
    ws.onerror = () => this.failAll(new RpcError(-32000, "websocket error"));
  }

  /** Connect to a `worldos serve` endpoint (default ws://127.0.0.1:7799). */
  static async connect(url = "ws://127.0.0.1:7799"): Promise<WorldosClient> {
    const ws = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      ws.onopen = () => resolve();
      ws.onerror = () => reject(new RpcError(-32000, `cannot connect to ${url}`));
    });
    return new WorldosClient(ws);
  }

  close(): void {
    this.ws.close();
  }

  /** Raw JSON-RPC call — the escape hatch for every method. */
  call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    const id = this.nextId++;
    const frame = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new RpcError(-32000, `timeout calling ${method}`));
      }, this.timeoutMs);
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timer,
      });
      this.ws.send(frame);
    });
  }

  private onMessage(ev: MessageEvent): void {
    let msg: { id?: number; result?: unknown; error?: { code: number; message: string; data?: unknown } };
    try {
      msg = JSON.parse(String(ev.data));
    } catch {
      return;
    }
    if (msg.id == null) return;
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    clearTimeout(p.timer);
    if (msg.error) p.reject(new RpcError(msg.error.code, msg.error.message, msg.error.data));
    else p.resolve(msg.result);
  }

  private failAll(e: RpcError): void {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(e);
    }
    this.pending.clear();
  }

  // ----- typed convenience API ----------------------------------------

  /** Execute a command (undoable transaction). */
  command(commandType: string, input: unknown = {}): Promise<CommandReceipt> {
    return this.call("command.execute", { type: commandType, input });
  }

  /** Run a capability (inspect, search, measure, agent.run…). */
  capability<T = unknown>(id: string, input: unknown = {}): Promise<T> {
    return this.call("capability.execute", { id, input });
  }

  objectGet(idOrName: string): Promise<WsObject> {
    const params = /^[0-9A-Z]{26}$/i.test(idOrName) ? { id: idOrName } : { name: idOrName };
    return this.call("object.get", params);
  }

  objectList(type?: string): Promise<{ objects: { id: string; name: string; type: string }[] }> {
    return this.call("object.list", type ? { type } : {});
  }

  search(q: { text?: string; type_id?: string; tag?: string; limit?: number }): Promise<unknown> {
    return this.call("project.search", q);
  }

  graph(): Promise<GraphView> {
    return this.call("project.graph");
  }

  commandList(): Promise<CommandSchema[]> {
    return this.call("command.list");
  }

  capabilityList(): Promise<CapabilityDescriptor[]> {
    return this.call("capability.list");
  }

  validate(): Promise<ValidationReport> {
    return this.call("validation.run");
  }

  history(limit = 50): Promise<{ transactions: TransactionInfo[] }> {
    return this.call("history.list", { limit });
  }

  undo(): Promise<{ undone: string | null }> {
    return this.call("history.undo");
  }

  redo(): Promise<{ redone: string | null }> {
    return this.call("history.redo");
  }

  save(path?: string): Promise<{ ok: boolean }> {
    return this.call("project.save", path ? { path } : {});
  }

  /** Run the builtin agent on a goal — returns its full report. */
  agentRun(goal: string, agent = "sdk-agent"): Promise<AgentReport> {
    return this.call("agent.run", { goal, agent });
  }

  info(): Promise<{
    id: string;
    name: string;
    object_count: number;
    relation_count: number;
    can_undo: boolean;
    can_redo: boolean;
  }> {
    return this.call("project.info");
  }
}

export * from "./types.js";
