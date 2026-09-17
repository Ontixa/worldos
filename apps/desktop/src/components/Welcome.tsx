import { useState } from "react";
import { api } from "../api";
import type { ProjectInfo } from "../types";

export default function Welcome({ onOpened }: { onOpened: (i: ProjectInfo) => void }) {
  const [path, setPath] = useState("world.worldos");
  const [name, setName] = useState("untitled");
  const [err, setErr] = useState("");

  const open = async () => {
    try {
      onOpened(await api.projectOpen(path));
    } catch (e) {
      setErr(String(e));
    }
  };
  const create = async () => {
    try {
      onOpened(await api.projectNew(name, path));
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="welcome">
      <h1>WorldOS</h1>
      <p className="dim">One world in which every tool operates.</p>
      <label>
        Project file (.worldos)
        <input value={path} onChange={(e) => setPath(e.target.value)} />
      </label>
      <label>
        New project name
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <div className="row">
        <button onClick={() => void open()}>Open</button>
        <button className="primary" onClick={() => void create()}>Create</button>
      </div>
      {err && <pre className="error">{err}</pre>}
    </div>
  );
}
