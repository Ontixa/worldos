#!/usr/bin/env python3
"""worldos-plugin-stamp — example hosted WorldOS plugin.

Speaks protocol v1 (line-delimited JSON-RPC; the plugin is the CLIENT):
requests on stdout, responses on stdin, diagnostics on stderr.

Uses the SDK's `worldos.Plugin` client when importable (installed SDK or
repo checkout); otherwise falls back to a minimal inline client so the
plugin works copied anywhere.

The sidecar manifest `worldos-plugin-stamp.json` declares the exact
permissions this plugin needs; the host grants nothing more.

Every mutation lands inside ONE transaction attributed to
`plugin:stamp` — non-zero exit or timeout rolls it all back.

Run it:  worldos plugin run my.worldos stamp
     or: worldos plugin run my.worldos examples/plugins/worldos-plugin-stamp.py
"""

import json
import os
import sys

for candidate in (
    os.path.join(os.path.dirname(__file__), "..", "..", "sdks", "worldos-py"),
    os.path.join(os.path.dirname(__file__), "..", "sdks", "worldos-py"),
):
    if os.path.isfile(os.path.join(candidate, "worldos.py")):
        sys.path.insert(0, candidate)
        break

try:
    import worldos

    Plugin = worldos.Plugin
except ImportError:

    class Plugin:  # minimal inline client — same call shape as the SDK
        def __init__(self):
            if not os.environ.get("WORLDOS_PROTOCOL"):
                raise RuntimeError("not inside a plugin session")
            self.name = os.environ.get("WORLDOS_PLUGIN_NAME", "plugin")
            self.project_path = os.environ.get("WORLDOS_PROJECT")
            self._id = 0

        def call(self, method, params=None):
            self._id += 1
            print(json.dumps({"id": self._id, "method": method,
                              "params": params or {}}), flush=True)
            resp = json.loads(sys.stdin.readline())
            if "error" in resp:
                raise RuntimeError(resp["error"]["message"])
            return resp["result"]

        def info(self):
            return self.call("project.info")

        def command(self, command_type, input=None):
            return self.call("command.execute",
                             {"type": command_type, "input": input or {}})

        def validate(self):
            return self.call("validation.run")

        def run(self, fn):
            fn(self)
            self.call("session.end")


def main(p):
    log = lambda *a: print(*a, file=sys.stderr)
    log(f"[stamp] protocol on {p.project_path or '<memory>'}")

    info = p.info()
    log(f"[stamp] project `{info['name']}` has {info['object_count']} objects")

    # create a stamped note — undoable as part of the plugin transaction
    p.command(
        "object.create",
        {
            "type": "core:note",
            "name": "plugin-stamp",
            "components": {
                "doc:text": {"text": "stamped by worldos-plugin-stamp"}
            },
        },
    )

    report = p.validate()
    log(f"[stamp] validation passed: {report['passed']}")


if __name__ == "__main__":
    try:
        Plugin().run(main)
    except Exception as e:
        print(f"[stamp] {e}", file=sys.stderr)
        sys.exit(1)
