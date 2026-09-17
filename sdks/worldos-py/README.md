# worldos (Python SDK)

Drive a WorldOS project from Python — zero dependencies. Spawns
`worldos rpc <file>` and speaks newline-delimited JSON-RPC.

```bash
pip install -e sdks/worldos-py   # or add the dir to PYTHONPATH
```

```python
import worldos

with worldos.open("genesis.worldos") as p:
    p.command("object.create", {
        "type": "core:note",
        "name": "from-python",
        "components": {"doc:text": {"text": "hello"}},
    })
    report = p.agent_run("create another cube next to reference-cube named housing")
    for step in report["steps"]:
        print(step["command"], "—", step["note"])
    p.undo()
    p.save()
```

All mutations are transactional commands: attributed, undoable, recorded
in project history. See `worldos commands <file>` for the catalog.
