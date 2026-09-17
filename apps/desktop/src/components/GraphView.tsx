import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { GraphView as G } from "../types";

/** SVG graph of the UPG: nodes on a layered radial layout, typed edges. */
export default function GraphView({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const [g, setG] = useState<G | null>(null);
  const [pan, setPan] = useState({ x: 0, y: 0, k: 1 });
  const drag = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    api.graph().then(setG).catch(() => setG(null));
  });

  if (!g) return <div className="viewport dim">loading graph…</div>;

  const n = g.nodes.length;
  const R = Math.max(140, n * 22);
  const pos = new Map<string, [number, number]>();
  g.nodes.forEach((node, i) => {
    const a = (i / Math.max(n, 1)) * Math.PI * 2 - Math.PI / 2;
    pos.set(node.id, [400 + R * Math.cos(a), 300 + R * Math.sin(a)]);
  });

  return (
    <svg
      className="viewport graphview"
      viewBox="0 0 800 600"
      onMouseDown={(e) => { drag.current = { x: e.clientX, y: e.clientY }; }}
      onMouseMove={(e) => {
        if (!drag.current) return;
        const dx = e.clientX - drag.current.x, dy = e.clientY - drag.current.y;
        drag.current = { x: e.clientX, y: e.clientY };
        setPan((p) => ({ ...p, x: p.x + dx / p.k, y: p.y + dy / p.k }));
      }}
      onMouseUp={() => { drag.current = null; }}
      onWheel={(e) => setPan((p) => ({ ...p, k: Math.min(3, Math.max(0.3, p.k * (e.deltaY > 0 ? 0.9 : 1.1))) }))}
    >
      <g transform={`translate(${pan.x},${pan.y}) scale(${pan.k})`}>
        {g.edges.map((e) => {
          const a = pos.get(e.from), b = pos.get(e.to);
          if (!a || !b) return null;
          const mx = (a[0] + b[0]) / 2, my = (a[1] + b[1]) / 2;
          return (
            <g key={e.id}>
              <line x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} stroke="#30363d" />
              <text x={mx} y={my} fill="#484f58" fontSize={9} textAnchor="middle">
                {e.type.split(":").pop()}
              </text>
            </g>
          );
        })}
        {g.nodes.map((node) => {
          const [x, y] = pos.get(node.id)!;
          const sel = node.id === selectedId;
          return (
            <g key={node.id} onClick={() => onSelect(node.id)} style={{ cursor: "pointer" }}>
              <circle cx={x} cy={y} r={18} fill={sel ? "#ffd33d" : "#1f6feb"} stroke="#30363d" />
              <text x={x} y={y + 32} fill="#c9d1d9" fontSize={11} textAnchor="middle">
                {node.name}
              </text>
              <text x={x} y={y + 44} fill="#484f58" fontSize={9} textAnchor="middle">
                {node.type.split(":").pop()}
              </text>
            </g>
          );
        })}
      </g>
    </svg>
  );
}
