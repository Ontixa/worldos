import { useEffect, useState } from "react";
import { api } from "../api";
import type { WsObject } from "../types";

/** Parameter controls for a cad:body — every edit is `cad.set_param`. */
function CadPanel({ object, onChanged }: { object: WsObject; onChanged: () => void }) {
  const op = object.components["cad:operation"]?.data;
  const shape = object.components["cad:shape"]?.data;
  const [params, setParams] = useState<Record<string, any>>(op?.params ?? {});
  const [sources, setSources] = useState<string[]>([]);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setParams(op?.params ?? {});
    setErr("");
    // feature-tree lineage: derived-from edges point at sources
    void api
      .objectRelations(object.id)
      .then((rels) =>
        setSources(
          rels.filter((r) => r.type === "core:derived-from" && r.from === object.id).map((r) => r.to),
        ),
      )
      .catch(() => setSources([]));
  }, [object.id, object.meta.revision]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!op || !shape) return null;
  const stale = !!shape.stale;
  const m = shape.measures;

  // editable scalar/array-of-scalar params; object refs + ids stay read-only
  const editable = Object.entries(params).filter(
    ([, v]) => typeof v === "number" || (Array.isArray(v) && v.every((x) => typeof x === "number")),
  );
  const refs = Object.entries(params).filter(
    ([k]) => !editable.some(([ek]) => ek === k),
  );

  const apply = async () => {
    setBusy(true);
    setErr("");
    try {
      const patch: Record<string, any> = {};
      for (const [k, v] of editable) {
        patch[k] = Array.isArray(v) ? v.map(Number) : Number(v);
      }
      await api.commandExecute("cad.set_param", { object: object.id, params: patch });
      onChanged();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const regen = async (all: boolean) => {
    setBusy(true);
    setErr("");
    try {
      await api.commandExecute(
        "cad.regenerate",
        all ? { all_stale: true } : { object: object.id, cascade: true },
      );
      onChanged();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="cadpanel">
      <h4>
        CAD · {op.kind} <span className="dim">({op.kernel})</span>
      </h4>
      {stale && <div className="badge stale">⟳ stale — inputs changed, regenerate</div>}
      {editable.map(([k, v]) => (
        <div className="prow" key={k}>
          <label>{k}</label>
          {Array.isArray(v) ? (
            <div className="vec">
              {v.map((x: number, i: number) => (
                <input
                  key={i}
                  type="number"
                  step="any"
                  value={x}
                  onChange={(e) =>
                    setParams((p) => {
                      const arr = [...(p[k] as number[])];
                      arr[i] = Number(e.target.value);
                      return { ...p, [k]: arr };
                    })
                  }
                />
              ))}
            </div>
          ) : (
            <input
              type="number"
              step="any"
              value={v}
              onChange={(e) => setParams((p) => ({ ...p, [k]: Number(e.target.value) }))}
            />
          )}
        </div>
      ))}
      {refs.length > 0 && (
        <div className="dim small">
          sources: {refs.map(([k, v]) => `${k}=${String(v).slice(0, 8)}`).join(" ")}
        </div>
      )}
      {sources.length > 0 && (
        <div className="dim small">derived-from: {sources.map((s) => s.slice(0, 8)).join(", ")}</div>
      )}
      <div className="row">
        <button disabled={busy} onClick={() => void apply()}>Apply params</button>
        <button disabled={busy} onClick={() => void regen(false)} title="regenerate this + dependents">
          Regen ▼
        </button>
        <button disabled={busy} onClick={() => void regen(true)} title="regenerate all stale bodies">
          Regen all stale
        </button>
      </div>
      {m && (
        <div className="dim small measures">
          <div>volume {m.volume_mm3?.toFixed(2)} mm³ · area {m.area_mm2?.toFixed(2)} mm²</div>
          <div>
            bbox [{m.bbox?.min_mm?.map((x: number) => x.toFixed(1)).join(", ")}] → [
            {m.bbox?.max_mm?.map((x: number) => x.toFixed(1)).join(", ")}] mm
          </div>
          <div>
            topo {shape.topology?.faces}F/{shape.topology?.edges}E
            {shape.topology?.is_valid ? " · valid" : " · INVALID"}
          </div>
          <div title={shape.brep}>brep {String(shape.brep).slice(0, 19)}…</div>
        </div>
      )}
      {err && <pre className="error">{err}</pre>}
    </div>
  );
}

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

      {object.type_id === "cad:body" && <CadPanel object={object} onChanged={onChanged} />}

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
