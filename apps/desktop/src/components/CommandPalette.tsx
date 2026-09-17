import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { CommandSchema } from "../types";

/** ⌘K palette — live command catalog from the engine's schemas. */
export default function CommandPalette({
  onClose,
  onExecuted,
}: {
  onClose: () => void;
  onExecuted: () => void;
}) {
  const [schemas, setSchemas] = useState<CommandSchema[]>([]);
  const [filter, setFilter] = useState("");
  const [sel, setSel] = useState(0);
  const [picked, setPicked] = useState<CommandSchema | null>(null);
  const [input, setInput] = useState("{}");
  const [err, setErr] = useState("");
  const box = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.commandList().then(setSchemas).catch(() => {});
    box.current?.focus();
  }, []);

  const matches = useMemo(() => {
    const f = filter.toLowerCase();
    return schemas.filter(
      (s) =>
        s.command_type.toLowerCase().includes(f) ||
        s.description.toLowerCase().includes(f) ||
        s.category.toLowerCase().includes(f),
    );
  }, [schemas, filter]);

  const pick = (s: CommandSchema) => {
    setPicked(s);
    setInput(template(s.input_schema));
  };

  const run = async () => {
    if (!picked) return;
    try {
      await api.commandExecute(picked.command_type, JSON.parse(input || "{}"));
      onExecuted();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        {!picked ? (
          <>
            <input
              ref={box}
              placeholder="Type a command…"
              value={filter}
              onChange={(e) => { setFilter(e.target.value); setSel(0); }}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") setSel((s) => Math.min(s + 1, matches.length - 1));
                if (e.key === "ArrowUp") setSel((s) => Math.max(s - 1, 0));
                if (e.key === "Enter" && matches[sel]) pick(matches[sel]);
                if (e.key === "Escape") onClose();
              }}
            />
            <ul>
              {matches.map((s, i) => (
                <li
                  key={s.command_type}
                  className={i === sel ? "sel" : ""}
                  onMouseEnter={() => setSel(i)}
                  onClick={() => pick(s)}
                >
                  <b>{s.command_type}</b>
                  <span className="dim"> {s.description}</span>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <>
            <h4>{picked.command_type}</h4>
            <p className="dim">{picked.description} · permission: {picked.permission}</p>
            <textarea
              autoFocus
              rows={8}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              spellCheck={false}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) void run();
                if (e.key === "Escape") setPicked(null);
              }}
            />
            {err && <pre className="error">{err}</pre>}
            <div className="row">
              <button className="primary" onClick={() => void run()}>Run (Ctrl+Enter)</button>
              <button onClick={() => setPicked(null)}>Back</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Build a starter JSON body from the command's input schema. */
function template(schema: any): string {
  const out: Record<string, unknown> = {};
  const props = schema?.properties ?? {};
  const req: string[] = schema?.required ?? [];
  for (const [k, v] of Object.entries<any>(props)) {
    if (!req.includes(k)) continue;
    out[k] =
      v.type === "string" ? (v.enum?.[0] ?? "") :
      v.type === "number" || v.type === "integer" ? 0 :
      v.type === "boolean" ? false :
      v.type === "array" ? [] : {};
  }
  return JSON.stringify(out, null, 2);
}
