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
