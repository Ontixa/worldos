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

## Writing a hosted plugin

The same module speaks the *other* side of the wire too. A
`worldos-plugin-*` executable is spawned by the host and gets a JSON-RPC
channel on its own stdin/stdout — `worldos.Plugin` is the client for
that side:

```python
import worldos

def main(p: worldos.Plugin):
    print(p.info()["object_count"], "objects")
    p.command("object.create", {"type": "core:note", "name": "hi"})

if __name__ == "__main__":
    worldos.Plugin().run(main)   # ends session → host commits one txn
```

Drop it in a `plugins/` dir (see `examples/plugins/README.md`), add a
`<stem>.json` manifest to declare exact permissions, and run it with
`worldos plugin run <file> <name>`.
