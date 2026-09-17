# WorldOS plugins

Hosted plugins are ordinary executables named `worldos-plugin-*` discovered in:

- `<project dir>/plugins/`
- `./plugins/`
- `~/.worldos/plugins/` (`%USERPROFILE%\.worldos\plugins` on Windows)
- any dir in `WORLDOS_PLUGIN_PATH`
- `PATH`

Run one:

```bash
worldos plugin list                 # discover
worldos plugin run my.worldos stamp # resolves worldos-plugin-stamp
```

## How it works

The host spawns the plugin and serves a **line-delimited JSON-RPC** channel
over the plugin's stdin/stdout. The plugin is the *client*:

```
plugin → stdout  {"id":1,"method":"command.execute","params":{"type":"object.create","input":{...}}}
host   → stdin   {"id":1,"result":{"id":"01J…"}}
plugin → stdout  {"id":2,"method":"session.end"}
plugin exits 0   → host commits ONE transaction attributed to `plugin:<name>`
```

Everything the plugin does flows through commands — same schemas, same
permissions, same undo/redo as a human or agent. Non-zero exit, protocol
violation, or timeout → the whole session rolls back.

## Plugin protocol v1

| method            | purpose                                        |
|-------------------|------------------------------------------------|
| `project.info`    | id, name, object/relation counts               |
| `object.list`     | `{type?}` → id/name/type list                  |
| `object.get`      | `{id|name}` → full object                      |
| `project.search`  | `SearchQuery` → matching objects               |
| `command.list`    | every registered command schema                |
| `command.execute` | `{type, input}` → run a command (mutations!)   |
| `validation.run`  | run validators, get diagnostics                |
| `session.end`     | finish cleanly                                 |

Env: `WORLDOS_PROJECT`, `WORLDOS_PLUGIN_NAME`, `WORLDOS_PROTOCOL=1`.
stderr is for diagnostics — keep stdout protocol-clean.

## Reference implementations

- `worldos-plugin-stamp.py` (this dir) — minimal Python plugin
- `worldos plugin-shim` — the CLI's own binary doubles as a Rust reference
  plugin (hidden subcommand; used by the e2e test)

Script extensions are dispatched to interpreters automatically:
`.py` → `python`, `.ps1` → `powershell -File`, `.cmd`/`.bat` → `cmd /c`.
