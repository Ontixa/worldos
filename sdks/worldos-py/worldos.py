"""WorldOS Python SDK — drive a project through `worldos rpc` (JSON-RPC over stdio).

Zero dependencies. Spawns the worldos CLI bound to a project file and sends
newline-delimited JSON-RPC — the same command layer the desktop app uses.

    import worldos
    p = worldos.open("robot.worldos")
    p.command("object.create", {"type": "core:note", "name": "spec"})
    report = p.agent_run("create another cube next to reference-cube named housing")
    p.undo()   # revert the whole agent transaction
    p.save()
    p.close()
"""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import dataclass
from typing import Any, Optional


class RpcError(Exception):
    def __init__(self, code: int, message: str, data: Any = None):
        super().__init__(f"[{code}] {message}")
        self.code = code
        self.data = data


@dataclass
class CommandReceipt:
    command_id: str
    transaction_id: str
    output: Any


def _find_binary() -> str:
    exe = shutil.which("worldos") or shutil.which("worldos.exe")
    if not exe:
        raise RpcError(-32000, "worldos binary not on PATH — run `cargo install --path crates/worldos-cli`")
    return exe


class Project:
    """A live handle on a .worldos project via a `worldos rpc` subprocess."""

    def __init__(self, proc: subprocess.Popen):
        self._proc = proc
        self._next_id = 0
        assert proc.stdin and proc.stdout

    # ----- transport ----------------------------------------------------

    def _call(self, method: str, params: Optional[dict] = None) -> Any:
        self._next_id += 1
        req = {"jsonrpc": "2.0", "id": self._next_id, "method": method,
               "params": params or {}}
        self._proc.stdin.write(json.dumps(req) + "\n")  # type: ignore[union-attr]
        self._proc.stdin.flush()                        # type: ignore[union-attr]
        while True:
            line = self._proc.stdout.readline()         # type: ignore[union-attr]
            if not line:
                raise RpcError(-32000, "worldos rpc subprocess closed stdout")
            resp = json.loads(line)
            if resp.get("id") != self._next_id:
                continue  # skip stray notifications
            if resp.get("error"):
                err = resp["error"]
                raise RpcError(err["code"], err["message"], err.get("data"))
            return resp.get("result")

    def close(self) -> None:
        if self._proc.poll() is None:
            self._proc.terminate()

    def __enter__(self) -> "Project":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # ----- typed API -----------------------------------------------------

    def info(self) -> dict:
        return self._call("project.info")

    def command(self, command_type: str, input: Optional[dict] = None) -> CommandReceipt:
        r = self._call("command.execute", {"type": command_type, "input": input or {}})
        return CommandReceipt(r["command_id"], r["transaction_id"], r["output"])

    def capability(self, cap_id: str, input: Optional[dict] = None) -> Any:
        return self._call("capability.execute", {"id": cap_id, "input": input or {}})

    def get(self, id_or_name: str) -> dict:
        key = "id" if len(id_or_name) == 26 else "name"
        return self._call("object.get", {key: id_or_name})

    def search(self, text: str = "", type_id: str = "", tag: str = "",
               has_component: str = "", limit: int = 50) -> list:
        q: dict = {"limit": limit}
        for k, v in (("text", text), ("type_id", type_id),
                     ("tag", tag), ("has_component", has_component)):
            if v:
                q[k] = v
        return self._call("project.search", q)["results"]

    def graph(self) -> dict:
        return self._call("project.graph")

    def validate(self) -> dict:
        return self._call("validation.run")

    def history(self, limit: int = 50) -> list:
        return self._call("history.list", {"limit": limit})["transactions"]

    def undo(self) -> Optional[str]:
        return self._call("history.undo")["undone"]

    def redo(self) -> Optional[str]:
        return self._call("history.redo")["redone"]

    def save(self, path: str = "") -> None:
        self._call("project.save", {"path": path} if path else {})

    def agent_run(self, goal: str, agent: str = "sdk-agent") -> dict:
        """Run the builtin agent; returns its structured report."""
        return self._call("agent.run", {"goal": goal, "agent": agent})


def open(path: str, binary: Optional[str] = None) -> Project:  # noqa: A001
    """Open an existing .worldos project (spawns `worldos rpc <path>`)."""
    exe = binary or _find_binary()
    proc = subprocess.Popen(
        [exe, "rpc", path],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
    )
    return Project(proc)


def create(name: str, path: str, binary: Optional[str] = None) -> Project:
    """Create a new project file and return a live handle."""
    exe = binary or _find_binary()
    subprocess.run([exe, "new", name, "--path", path],
                   check=True, capture_output=True, text=True)
    return open(path, exe)


# ---------------------------------------------------------------------------
# Plugin side — the other end of the same protocol.
#
# A `worldos-plugin-*` executable is spawned by the host and speaks
# line-delimited JSON-RPC over its OWN stdin/stdout (requests on stdout,
# responses on stdin). Everything runs as the `plugin:<name>` actor inside
# one transaction — a clean exit commits, a crash rolls back.
#
#   def main(p):
#       p.command("object.create", {"type": "core:note", "name": "hi"})
#
#   if __name__ == "__main__":
#       worldos.Plugin().run(main)
# ---------------------------------------------------------------------------


class PluginError(Exception):
    pass


class Plugin:
    """Client handle for code running INSIDE a hosted plugin process."""

    def __init__(self):
        import os
        import sys

        if not os.environ.get("WORLDOS_PROTOCOL"):
            raise PluginError(
                "not inside a plugin session — run via `worldos plugin run`"
            )
        self.name = os.environ.get("WORLDOS_PLUGIN_NAME", "plugin")
        self.project_path = os.environ.get("WORLDOS_PROJECT")
        self._in = sys.stdin
        self._out = sys.stdout
        self._next_id = 0

    def call(self, method: str, params: Optional[dict] = None) -> Any:
        self._next_id += 1
        self._out.write(
            json.dumps({"id": self._next_id, "method": method,
                        "params": params or {}}) + "\n"
        )
        self._out.flush()
        line = self._in.readline()
        if not line:
            raise PluginError("host closed the session channel")
        resp = json.loads(line)
        if resp.get("error"):
            raise PluginError(resp["error"].get("message", "plugin call failed"))
        return resp.get("result")

    def info(self) -> dict:
        return self.call("project.info")

    def objects(self, type_id: str = "") -> list:
        params = {"type": type_id} if type_id else {}
        return self.call("object.list", params)["objects"]

    def get(self, id_or_name: str) -> dict:
        key = "id" if len(id_or_name) == 26 else "name"
        return self.call("object.get", {key: id_or_name})

    def search(self, **query: Any) -> list:
        return self.call("project.search", query)["results"]

    def commands(self) -> list:
        return self.call("command.list")

    def command(self, command_type: str, input: Optional[dict] = None) -> Any:
        return self.call(
            "command.execute", {"type": command_type, "input": input or {}}
        )

    def validate(self) -> dict:
        return self.call("validation.run")

    def end(self) -> None:
        """Signal a clean finish — the host commits the transaction."""
        self.call("session.end")

    def run(self, fn) -> None:
        """Run `fn(plugin)`, then end the session. Exceptions propagate —
        the host sees a crash and rolls back the transaction."""
        fn(self)
        self.end()
