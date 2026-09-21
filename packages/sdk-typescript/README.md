# @worldos/sdk

TypeScript SDK for WorldOS. Talks JSON-RPC to a running `worldos serve`
instance — the same command layer the desktop app and MCP server use.

```bash
worldos serve myproject.worldos --port 7799
```

```ts
import { WorldosClient } from "@worldos/sdk";

const w = await WorldosClient.connect("ws://127.0.0.1:7799");

const note = await w.command("object.create", {
  type: "core:note",
  name: "sdk-note",
  components: { "doc:text": { text: "hello from the SDK" } },
});

const report = await w.agentRun("create another cube next to reference-cube named housing");
console.log(report.steps); // the exact commands the agent executed

await w.undo();            // revert the whole agent transaction
await w.save();
w.close();
```

Reads are free; mutations are commands — transactional, undoable,
attributed. See `worldos commands <file>` for the command catalog.

## Client lifecycle and local tests

Calls use increasing numeric IDs, with a 30-second response timeout. Responses
may arrive out of order. RPC errors preserve their code, message and optional
data as `RpcError`; connection close/error rejects pending calls. A synchronous
WebSocket send failure preserves its original rejection and clears request
bookkeeping. Invalid JSON, null/non-object frames, notifications and unknown
response IDs are ignored. This is not full runtime validation of RPC payloads,
an automatic reconnect/retry policy, or a server authentication boundary.

Run `npm run test -w @worldos/sdk` from the repository root. Tests use an in-memory
fake WebSocket and fake timers: they cover client frames/correlation, lifecycle
cleanup and convenience-method mapping, not a real server or native CAD engine.
Tests live outside `src` and are not emitted into the SDK's `dist` package.
