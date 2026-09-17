#!/usr/bin/env python3
"""worldos-plugin-stamp — example hosted WorldOS plugin.

Protocol v1 (line-delimited JSON-RPC; the plugin is the CLIENT):
  write requests  → stdout   {"id": N, "method": ..., "params": {...}}
  read responses  ← stdin    {"id": N, "result": ...} | {"id": N, "error": {...}}
  diagnostics     → stderr   (never stdout)
  finish          → {"id": N, "method": "session.end"} then exit 0

Env provided by the host: WORLDOS_PROJECT, WORLDOS_PLUGIN_NAME,
WORLDOS_PROTOCOL.

Every mutation lands inside ONE transaction attributed to
`plugin:stamp` — if this process exits non-zero or times out,
the host rolls back all of its work.

Run it:  worldos plugin run my.worldos stamp
     or: worldos plugin run my.worldos examples/plugins/worldos-plugin-stamp.py
"""

import json
import os
import sys

_id = 0


def call(method, params=None):
    """Send one request, wait for its response."""
    global _id
    _id += 1
    print(json.dumps({"id": _id, "method": method, "params": params or {}}),
          flush=True)
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError("host closed the channel")
    resp = json.loads(line)
    if "error" in resp:
        raise RuntimeError(resp["error"]["message"])
    return resp["result"]


def main():
    log = lambda *a: print(*a, file=sys.stderr)
    log(f"[stamp] protocol v{os.environ.get('WORLDOS_PROTOCOL')} "
        f"on {os.environ.get('WORLDOS_PROJECT', '<memory>')}")

    info = call("project.info")
    log(f"[stamp] project `{info['name']}` has {info['object_count']} objects")

    # create a stamped note — undoable as part of the plugin transaction
    call("command.execute", {
        "type": "object.create",
        "input": {
            "type": "core:note",
            "name": "plugin-stamp",
            "components": {
                "doc:text": {"text": "stamped by worldos-plugin-stamp"}
            },
        },
    })

    report = call("validation.run")
    log(f"[stamp] validation passed: {report['passed']}")

    call("session.end")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        print(f"[stamp] {e}", file=sys.stderr)
        sys.exit(1)
