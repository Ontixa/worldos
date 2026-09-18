import { useEffect, useRef, useState } from "react";
import type { WsObject } from "../types";

/** Minimal software renderer for `geom:*` objects. Orbit: drag, zoom: wheel. */

type V3 = [number, number, number];

interface Prim {
  id: string;
  name: string;
  kind: string;
  size: number[];
  pos: V3;
  scale: V3;
  color: string;
}

function toPrim(o: WsObject): Prim | null {
  const g = o.components["geom:geometry"]?.data;
  if (!g) return null;
  const t = o.components["core:transform"]?.data ?? {};
  const m = o.components["geom:material"]?.data ?? {};
  const v3 = (v: any, d: V3): V3 =>
    Array.isArray(v) ? [+v[0] || 0, +v[1] || 0, +v[2] || 0] : d;
  const size = Array.isArray(g.size) ? g.size.map(Number) : [Number(g.size) || 1, Number(g.size) || 1, Number(g.size) || 1];
  return {
    id: o.id,
    name: o.name,
    kind: String(g.kind ?? o.type_id.split(":").pop()),
    size,
    pos: v3(t.position, [0, 0, 0]),
    scale: v3(t.scale, [1, 1, 1]),
    color: typeof m.color === "string" ? m.color : "#7aa2f7",
  };
}

export default function Viewport3D({
  objects,
  selectedId,
  onSelect,
}: {
  objects: WsObject[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [cam, setCam] = useState({ yaw: -0.7, pitch: 0.55, dist: 14 });
  const drag = useRef<{ x: number; y: number } | null>(null);
  const prims = objects.map(toPrim).filter((p): p is Prim => p !== null);
  const hit = useRef<{ id: string; x: number; y: number; r: number }[]>([]);

  useEffect(() => {
    const canvas = ref.current!;
    const ctx = canvas.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const { clientWidth: w, clientHeight: h } = canvas;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);
    ctx.fillStyle = "#0d1117";
    ctx.fillRect(0, 0, w, h);

    const { yaw, pitch, dist } = cam;
    const cy = Math.cos(yaw), sy = Math.sin(yaw);
    const cp = Math.cos(pitch), sp = Math.sin(pitch);
    const f = Math.min(w, h) * 0.9;

    // camera transform → view space (x right, y up, z depth)
    const view = (p: V3): V3 => {
      const x = p[0] * cy - p[2] * sy;
      const z0 = p[0] * sy + p[2] * cy;
      const y = p[1] * cp - z0 * sp;
      const z = p[1] * sp + z0 * cp + dist;
      return [x, y, z];
    };
    const proj = (p: V3): [number, number, number] => {
      const [x, y, z] = view(p);
      const s = f / Math.max(z, 0.5);
      return [w / 2 + x * s, h / 2 - y * s, z];
    };

    // ground grid
    ctx.strokeStyle = "#21262d";
    ctx.lineWidth = 1;
    for (let i = -6; i <= 6; i++) {
      line(ctx, proj([i, 0, -6]), proj([i, 0, 6]));
      line(ctx, proj([-6, 0, i]), proj([6, 0, i]));
    }
    // axes
    ctx.strokeStyle = "#3d4459";
    line(ctx, proj([0, 0, 0]), proj([3, 0, 0]));
    line(ctx, proj([0, 0, 0]), proj([0, 3, 0]));
    line(ctx, proj([0, 0, 0]), proj([0, 0, 3]));

    hit.current = [];
    const sorted = [...prims].sort((a, b) => view(b.pos)[2] - view(a.pos)[2]);
    for (const p of sorted) {
      const sel = p.id === selectedId;
      const [cx, cy2, z] = proj(p.pos);
      const scalePix = f / Math.max(z, 0.5);
      const dims = p.size.map((s, i) => s * p.scale[i]);
      const rad = (Math.max(...dims) / 2) * scalePix;

      if (p.kind === "sphere") {
        ctx.beginPath();
        ctx.arc(cx, cy2, Math.max(rad, 3), 0, Math.PI * 2);
        ctx.fillStyle = shade(p.color, sel);
        ctx.fill();
        ctx.strokeStyle = sel ? "#ffd33d" : "#30363d";
        ctx.stroke();
      } else if (p.kind === "cone") {
        // pyramid wireframe: square base + apex
        const [rx, , h2] = [dims[0] / 2, dims[1] / 2, dims[2] / 2];
        const base: V3[] = [
          [-rx, -h2, -rx], [rx, -h2, -rx], [rx, -h2, rx], [-rx, -h2, rx],
        ].map(([x, y, z]) => [x + p.pos[0], y + p.pos[1], z + p.pos[2]] as V3);
        const apex = proj([p.pos[0], p.pos[1] + h2, p.pos[2]]);
        const pb = base.map(proj);
        ctx.fillStyle = shade(p.color, sel);
        ctx.strokeStyle = sel ? "#ffd33d" : p.color;
        ctx.lineWidth = sel ? 2 : 1;
        poly(ctx, pb);
        for (const c of pb) line(ctx, c, apex);
      } else if (p.kind === "torus") {
        // torus lies flat in xz: outer ellipse (proj of circle) + hole hint
        ctx.strokeStyle = sel ? "#ffd33d" : p.color;
        ctx.fillStyle = shade(p.color, sel);
        ctx.lineWidth = sel ? 2 : 1;
        const ring = (r: number) => {
          ctx.beginPath();
          for (let i = 0; i <= 24; i++) {
            const a = (i / 24) * Math.PI * 2;
            const pt = proj([p.pos[0] + r * Math.cos(a), p.pos[1], p.pos[2] + r * Math.sin(a)]);
            i === 0 ? ctx.moveTo(pt[0], pt[1]) : ctx.lineTo(pt[0], pt[1]);
          }
          ctx.closePath();
          ctx.globalAlpha = 0.25;
          ctx.fill();
          ctx.globalAlpha = 1;
          ctx.stroke();
        };
        ring(dims[0] / 2);               // outer edge of ring
        ring(dims[0] / 2 - dims[1] / 2); // inner edge (hole)
      } else {
        // box-ish wireframe for cube/cylinder/plane
        const [sx2, sy2, sz2] = [dims[0] / 2, dims[1] / 2, dims[2] / 2];
        const h = p.kind === "plane" ? 0.02 : sy2;
        const corners: V3[] = [
          [-sx2, -h, -sz2], [sx2, -h, -sz2], [sx2, -h, sz2], [-sx2, -h, sz2],
          [-sx2, h, -sz2], [sx2, h, -sz2], [sx2, h, sz2], [-sx2, h, sz2],
        ].map(([x, y, z]) => [x + p.pos[0], y + p.pos[1], z + p.pos[2]] as V3);
        const pc = corners.map(proj);
        ctx.fillStyle = shade(p.color, sel);
        ctx.strokeStyle = sel ? "#ffd33d" : p.color;
        ctx.lineWidth = sel ? 2 : 1;
        poly(ctx, [pc[0], pc[1], pc[2], pc[3]]);   // bottom
        poly(ctx, [pc[4], pc[5], pc[6], pc[7]]);   // top
        ctx.beginPath();
        for (const [a, b] of [[0, 4], [1, 5], [2, 6], [3, 7]]) {
          ctx.moveTo(pc[a][0], pc[a][1]);
          ctx.lineTo(pc[b][0], pc[b][1]);
        }
        ctx.stroke();
      }
      // label + hit region
      ctx.fillStyle = sel ? "#ffd33d" : "#8b949e";
      ctx.font = "11px sans-serif";
      ctx.fillText(p.name, cx + rad + 4, cy2);
      hit.current.push({ id: p.id, x: cx, y: cy2, r: Math.max(rad, 8) });
    }
    if (prims.length === 0) {
      ctx.fillStyle = "#484f58";
      ctx.font = "13px sans-serif";
      ctx.fillText("No geometry yet — create one with ⌘ Command or the agent panel.", 20, 30);
    }
  });

  const pick = (e: React.MouseEvent) => {
    const r = ref.current!.getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    const h = hit.current.find((h2) => (x - h2.x) ** 2 + (y - h2.y) ** 2 <= h2.r ** 2);
    if (h) onSelect(h.id);
  };

  return (
    <canvas
      ref={ref}
      className="viewport"
      onMouseDown={(e) => { drag.current = { x: e.clientX, y: e.clientY }; }}
      onMouseMove={(e) => {
        if (!drag.current) return;
        const dx = e.clientX - drag.current.x, dy = e.clientY - drag.current.y;
        drag.current = { x: e.clientX, y: e.clientY };
        setCam((c) => ({ ...c, yaw: c.yaw + dx * 0.01, pitch: clamp(c.pitch + dy * 0.01, -1.4, 1.4) }));
      }}
      onMouseUp={pick}
      onMouseLeave={() => { drag.current = null; }}
      onWheel={(e) => setCam((c) => ({ ...c, dist: clamp(c.dist + e.deltaY * 0.02, 3, 60) }))}
    />
  );
}

function line(ctx: CanvasRenderingContext2D, a: number[], b: number[]) {
  ctx.beginPath();
  ctx.moveTo(a[0], a[1]);
  ctx.lineTo(b[0], b[1]);
  ctx.stroke();
}
function poly(ctx: CanvasRenderingContext2D, pts: number[][]) {
  ctx.beginPath();
  ctx.moveTo(pts[0][0], pts[0][1]);
  for (const p of pts.slice(1)) ctx.lineTo(p[0], p[1]);
  ctx.closePath();
  ctx.globalAlpha = 0.25;
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.stroke();
}
function shade(hex: string, sel: boolean) {
  if (sel) return "#ffd33d";
  return hex.startsWith("#") ? hex : "#7aa2f7";
}
function clamp(v: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, v));
}
