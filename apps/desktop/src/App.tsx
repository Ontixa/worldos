import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import type { ProjectInfo, TxnInfo, WsObject } from "./types";
import Welcome from "./components/Welcome";
import ObjectList from "./components/ObjectList";
import Viewport3D from "./components/Viewport3D";
import GraphView from "./components/GraphView";
import Inspector from "./components/Inspector";
import HistoryPanel from "./components/HistoryPanel";
import AgentPanel from "./components/AgentPanel";
import CommandPalette from "./components/CommandPalette";

export default function App() {
  const [info, setInfo] = useState<ProjectInfo | null>(null);
  const [objects, setObjects] = useState<WsObject[]>([]);
  const [history, setHistory] = useState<TxnInfo[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<"3d" | "graph">("3d");
  const [palette, setPalette] = useState(false);
  const [status, setStatus] = useState("");

  const refresh = useCallback(async () => {
    const i = await api.projectInfo();
    setInfo(i);
    if (i) {
      setObjects(await api.objectList());
      setHistory(await api.history(60));
    }
  }, []);

  useEffect(() => {
    refresh().catch((e) => setStatus(String(e)));
  }, [refresh]);

  // Global shortcuts: palette, save, undo/redo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      } else if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void api.projectSave().then(refresh).then(() => setStatus("saved"));
      } else if (mod && e.key.toLowerCase() === "z") {
        e.preventDefault();
        void (e.shiftKey ? api.redo() : api.undo()).then(refresh);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [refresh]);

  if (!info) {
    return <Welcome onOpened={(i) => { setInfo(i); void refresh(); }} />;
  }

  const selected = objects.find((o) => o.id === selectedId) ?? null;

  return (
    <div className="app">
      <header className="toolbar">
        <span className="brand">WorldOS</span>
        <span className="proj" title={info.path ?? ""}>
          {info.name}
          {info.dirty ? " •" : ""}
        </span>
        <div className="spacer" />
        <div className="viewtabs">
          <button className={view === "3d" ? "on" : ""} onClick={() => setView("3d")}>3D</button>
          <button className={view === "graph" ? "on" : ""} onClick={() => setView("graph")}>Graph</button>
        </div>
        <button disabled={!info.can_undo} onClick={() => void api.undo().then(refresh)} title="Ctrl+Z">
          Undo
        </button>
        <button disabled={!info.can_redo} onClick={() => void api.redo().then(refresh)} title="Ctrl+Shift+Z">
          Redo
        </button>
        <button onClick={() => void api.projectSave().then(refresh).then(() => setStatus("saved"))} title="Ctrl+S">
          Save
        </button>
        <button onClick={() => setPalette(true)} title="Ctrl+K">⌘ Command</button>
      </header>

      <div className="workspace">
        <aside className="left">
          <ObjectList objects={objects} selectedId={selectedId} onSelect={setSelectedId} />
        </aside>
        <main className="center">
          {view === "3d" ? (
            <Viewport3D objects={objects} selectedId={selectedId} onSelect={setSelectedId} />
          ) : (
            <GraphView selectedId={selectedId} onSelect={setSelectedId} />
          )}
        </main>
        <aside className="right">
          <Inspector object={selected} onChanged={refresh} />
          <HistoryPanel history={history} />
        </aside>
      </div>

      <AgentPanel onDone={refresh} />
      <footer className="statusbar">
        <span>{status || "ready"}</span>
        <span className="spacer" />
        <span>{info.object_count} objects · {info.relation_count} relations · actor {info.actor}</span>
      </footer>

      {palette && (
        <CommandPalette
          onClose={() => setPalette(false)}
          onExecuted={() => { setPalette(false); void refresh(); }}
        />
      )}
    </div>
  );
}
