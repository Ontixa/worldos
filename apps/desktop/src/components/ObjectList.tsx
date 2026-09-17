import type { WsObject } from "../types";

const ICONS: [RegExp, string][] = [
  [/geometry|primitive/i, "◆"],
  [/note|document|text/i, "≡"],
  [/code|file/i, "</>"],
  [/requirement/i, "✓"],
  [/decision/i, "?"],
  [/agent|task/i, "➤"],
];

export default function ObjectList({
  objects,
  selectedId,
  onSelect,
}: {
  objects: WsObject[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="panel">
      <h3>Objects <span className="dim">{objects.length}</span></h3>
      <ul className="objlist">
        {objects.map((o) => {
          const icon = ICONS.find(([re]) => re.test(o.type_id))?.[1] ?? "·";
          return (
            <li
              key={o.id}
              className={o.id === selectedId ? "sel" : ""}
              onClick={() => onSelect(o.id)}
              title={o.type_id}
            >
              <span className="ico">{icon}</span> {o.name}
              <span className="dim"> {shortType(o.type_id)}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function shortType(t: string) {
  return t.split(":").pop() ?? t;
}
