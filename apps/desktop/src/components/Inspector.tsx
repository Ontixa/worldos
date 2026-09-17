import { useState } from "react";
import { api } from "../api";
import type { WsObject } from "../types";

/** Inspector: shows the selected object's components; edits flow through commands. */
export default function Inspector({
  object,
  onChanged,
}: {
  object: WsObject | null;
  onChanged: () => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const [json, setJson] = useState("");
  const [err, setErr] = useState("");
  const [rename, setRename] = useState<string | null>(null);

  if (!object) {
    return (
      <div className="panel inspector">
        <h3>Inspector</h3>
        <p className="dim">Select an object.</p>
      </div>
    );
  }

  const saveComponent = async (ctype: string) => {
    try {
      const data = JSON.parse(json);
      await api.commandExecute("object.set_component", { id: object.id, component: ctype, data });
      setEditing(null);
      setErr("");
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
  };

  const doRename = async () => {
    if (rename == null) return;
    try {
      await api.commandExecute("object.rename", { id: object.id, name: rename });
      setRename(null);
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="panel inspector">
      <h3>Inspector</h3>
      {rename === null ? (
        <div className="objtitle" onDoubleClick={() => setRename(object.name)} title="double-click to rename">
          {object.name}
        </div>
      ) : (
        <input
          autoFocus
          value={rename}
          onChange={(e) => setRename(e.target.value)}
          onBlur={() => void doRename()}
          onKeyDown={(e) => e.key === "Enter" && void doRename()}
        />
      )}
      <div className="dim small">{object.type_id} · rev {object.meta.revision} · by {object.meta.updated_by}</div>

      {Object.entries(object.components).map(([ctype, comp]) => (
        <details key={ctype} open className="comp">
          <summary>{ctype} <span className="dim">v{comp.version}</span></summary>
          {editing === ctype ? (
            <>
              <textarea
                value={json}
                onChange={(e) => setJson(e.target.value)}
                rows={6}
                spellCheck={false}
              />
              <div className="row">
                <button onClick={() => void saveComponent(ctype)}>Apply</button>
                <button onClick={() => setEditing(null)}>Cancel</button>
              </div>
            </>
          ) : (
            <pre onClick={() => { setEditing(ctype); setJson(JSON.stringify(comp.data, null, 2)); }}>
              {JSON.stringify(comp.data, null, 2)}
            </pre>
          )}
        </details>
      ))}
      {object.tags.length > 0 && <div className="dim small">tags: {object.tags.join(", ")}</div>}
      {err && <pre className="error">{err}</pre>}
    </div>
  );
}
