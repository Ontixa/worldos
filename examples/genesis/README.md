# examples/genesis

A real `.worldos` project produced end-to-end by the CLI — the Genesis
acceptance scenario:

1. `worldos new genesis` — create the project file (SQLite)
2. `requirement.create` — an evaluable requirement
3. `object.create` — a note object (`doc:text` component)
4. `geometry.create_primitive` — a cube named `reference-cube`
5. `worldos agent … "create another cube next to reference-cube named housing"`
   — the builtin planner ran `geometry.create_primitive` +
   `geometry.transform` + `requirement.evaluate` in **one transaction**,
   attributed to the agent actor.

Inspect it:

```bash
worldos inspect genesis.worldos
worldos graph genesis.worldos
worldos history genesis.worldos
worldos undo genesis.worldos     # reverts the whole agent transaction
worldos redo genesis.worldos     # replays it
worldos validate genesis.worldos
```

Regenerate from scratch:

```bash
./examples/genesis/regenerate.sh   # requires `worldos` on PATH
```

Or open it in the desktop app / serve it for the SDKs:

```bash
worldos serve genesis.worldos --port 7799
```
