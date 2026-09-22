import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { RpcError, WorldosClient } from "../src/client.js";

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  sent: string[] = [];
  sendError?: Error;
  closed = false;
  constructor(readonly url: string) { FakeWebSocket.instances.push(this); }
  send(frame: string) {
    if (this.sendError) throw this.sendError;
    this.sent.push(frame);
  }
  open() { this.onopen?.(); }
  error() { this.onerror?.(); }
  close() { this.closed = true; this.onclose?.(); }
  raw(data: string) { this.onmessage?.({ data } as MessageEvent); }
  reply(frame: unknown) { this.raw(JSON.stringify(frame)); }
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function connected() {
  const connection = WorldosClient.connect();
  const socket = FakeWebSocket.instances[0];
  socket.open();
  return { client: await connection, socket };
}

// Inspect retained bookkeeping only to prove cleanup; production API stays unchanged.
function pendingCount(client: WorldosClient): number {
  return (Reflect.get(client, "pending") as Map<number, unknown>).size;
}

test("JSON null and non-object frames are ignored without throwing", async () => {
  const { client, socket } = await connected();
  const response = client.call("fixture");
  try {
    for (const value of [null, true, 42, "text", []]) {
      expect(() => socket.reply(value)).not.toThrow();
      expect(pendingCount(client)).toBe(1);
    }
  } finally {
    socket.reply({ id: 1, result: "ok" });
    await expect(response).resolves.toBe("ok");
  }
});

test("synchronous send failure rejects and clears timer and pending request", async () => {
  const { client, socket } = await connected();
  const failure = new Error("fixture send failure");
  socket.sendError = failure;
  await expect(client.call("fixture")).rejects.toBe(failure);
  expect(vi.getTimerCount()).toBe(0);
  expect(pendingCount(client)).toBe(0);
  socket.sendError = undefined;
  const next = client.call("next");
  socket.reply({ id: 2, result: "recovered" });
  await expect(next).resolves.toBe("recovered");
  expect(vi.getTimerCount()).toBe(0);
});

test("connect waits for open and forwards default and explicit URLs", async () => {
  for (const url of [undefined, "ws://fixture.invalid:1234"]) {
    let settled = false;
    const connection = WorldosClient.connect(url).then(client => { settled = true; return client; });
    const socket = FakeWebSocket.instances.at(-1)!;
    expect(socket.url).toBe(url ?? "ws://127.0.0.1:7799");
    await Promise.resolve();
    expect(settled).toBe(false);
    socket.open();
    const client = await connection;
    client.close();
    expect(socket.closed).toBe(true);
  }
});

test("connection errors reject with RpcError and constructor errors propagate", async () => {
  const connection = WorldosClient.connect("ws://fixture.invalid");
  const rejected = expect(connection).rejects.toMatchObject({ name: "RpcError", code: -32000, message: "cannot connect to ws://fixture.invalid" });
  FakeWebSocket.instances[0].error();
  await rejected;
  const failure = new Error("constructor failure");
  vi.stubGlobal("WebSocket", class { constructor() { throw failure; } });
  await expect(WorldosClient.connect()).rejects.toBe(failure);
});

test("raw calls use exact frames and correlate out-of-order responses", async () => {
  const { client, socket } = await connected();
  const first = client.call("first", { value: "literal" });
  const second = client.call("second");
  expect(socket.sent).toEqual([
    '{"jsonrpc":"2.0","id":1,"method":"first","params":{"value":"literal"}}',
    '{"jsonrpc":"2.0","id":2,"method":"second","params":{}}',
  ]);
  socket.reply({ jsonrpc: "2.0", id: 2, result: false });
  await expect(second).resolves.toBe(false);
  expect(pendingCount(client)).toBe(1);
  socket.reply({ jsonrpc: "2.0", id: 1, result: null });
  await expect(first).resolves.toBeNull();
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
});

test("RPC errors retain code message data and clear their pending request", async () => {
  const { client, socket } = await connected();
  const response = client.call("denied");
  const rejection = response.catch(error => error);
  socket.reply({ id: 1, error: { code: -32602, message: "bad input", data: { field: "size" } } });
  const error = await rejection;
  expect(error).toBeInstanceOf(RpcError);
  expect(error).toMatchObject({ name: "RpcError", code: -32602, message: "bad input", data: { field: "size" } });
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
});

test("notifications unknown IDs and invalid JSON do not settle a pending call", async () => {
  const { client, socket } = await connected();
  const response = client.call("fixture");
  socket.raw("not json");
  socket.reply({ method: "notification", params: {} });
  socket.reply({ id: 999, result: "unknown" });
  socket.reply({ id: "1", result: "wrong ID type" });
  expect(pendingCount(client)).toBe(1);
  expect(vi.getTimerCount()).toBe(1);
  socket.reply({ id: 1, result: "expected" });
  await expect(response).resolves.toBe("expected");
  socket.reply({ id: 1, result: "duplicate" });
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
});

test("timeout rejects at 30 seconds and late response cannot restore pending state", async () => {
  const { client, socket } = await connected();
  const response = client.call("slow");
  const rejection = expect(response).rejects.toMatchObject({ code: -32000, message: "timeout calling slow" });
  await vi.advanceTimersByTimeAsync(29_999);
  expect(pendingCount(client)).toBe(1);
  await vi.advanceTimersByTimeAsync(1);
  await rejection;
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
  socket.reply({ id: 1, result: "late" });
  expect(pendingCount(client)).toBe(0);
});

test.each(["close", "error"] as const)("socket %s rejects every pending call and clears timers", async event => {
  const { client, socket } = await connected();
  const responses = [client.call("one"), client.call("two")];
  const rejected = responses.map(response => expect(response).rejects.toMatchObject({
    code: -32000, message: event === "close" ? "connection closed" : "websocket error",
  }));
  if (event === "close") client.close(); else socket.error();
  await Promise.all(rejected);
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
  socket.reply({ id: 1, result: "late" });
  socket.close();
  expect(pendingCount(client)).toBe(0);
});

test("typed convenience methods map to exact RPC method and parameter shapes", async () => {
  const { client, socket } = await connected();
  const objectId = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => client.command("object.create"), "command.execute", { type: "object.create", input: {} }],
    [() => client.command("object.create", { name: "x" }), "command.execute", { type: "object.create", input: { name: "x" } }],
    [() => client.capability("inspect"), "capability.execute", { id: "inspect", input: {} }],
    [() => client.capability("inspect", { id: "x" }), "capability.execute", { id: "inspect", input: { id: "x" } }],
    [() => client.objectGet(objectId), "object.get", { id: objectId }],
    [() => client.objectGet("cube"), "object.get", { name: "cube" }],
    [() => client.objectList(), "object.list", {}],
    [() => client.objectList("cad:body"), "object.list", { type: "cad:body" }],
    [() => client.search({ text: "cube", limit: 3 }), "project.search", { text: "cube", limit: 3 }],
    [() => client.graph(), "project.graph", {}],
    [() => client.commandList(), "command.list", {}],
    [() => client.capabilityList(), "capability.list", {}],
    [() => client.validate(), "validation.run", {}],
    [() => client.history(), "history.list", { limit: 50 }],
    [() => client.history(2), "history.list", { limit: 2 }],
    [() => client.undo(), "history.undo", {}],
    [() => client.redo(), "history.redo", {}],
    [() => client.save(), "project.save", {}],
    [() => client.save("copy.worldos"), "project.save", { path: "copy.worldos" }],
    [() => client.agentRun("goal"), "agent.run", { goal: "goal", agent: "sdk-agent" }],
    [() => client.agentRun("goal", "fixture"), "agent.run", { goal: "goal", agent: "fixture" }],
    [() => client.info(), "project.info", {}],
  ];
  for (const [index, [invoke, method, params]] of cases.entries()) {
    const response = invoke();
    expect(JSON.parse(socket.sent[index])).toEqual({ jsonrpc: "2.0", id: index + 1, method, params });
    socket.reply({ id: index + 1, result: { fixture: index } });
    await expect(response).resolves.toEqual({ fixture: index });
  }
  expect(pendingCount(client)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
});
