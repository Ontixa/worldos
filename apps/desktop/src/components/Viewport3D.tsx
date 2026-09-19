import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { CadMesh, WsObject } from "../types";

/**
 * Software renderer. `cad:body` objects draw their REAL BRep
 * tessellation (kernel mesh of the stored artifact, mm, Z-up→Y-up).
 * `geom:*` objects draw the legacy primitive proxies.
 * Orbit: drag, zoom: wheel.
 */

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

/** A loaded CAD mesh ready to draw. */
interface Body {
  id: string;
  name: string;
  mesh: CadMesh;
}

// CAD is Z-up mm; the camera treats +Y as up — remap (x,y,z)→(x,z,y).
const zu = (p: number[]): V3 => [p[0], p[2] ?? 0, p[1]];

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
  const [bodies, setBodies] = useState<Map<string, Body>>(new Map());
  const [meshErr, setMeshErr] = useState<string>("");
  const drag = useRef<{ x: number; y: number } | null>(null);
  const hit = useRef<{ id: string; x: number; y: number; r: number }[]>([]);

  const cadObjs = objects.filter((o) => o.type_id === "cad:body");
  const prims = objects.map(toPrim).filter((p): p is Prim => p !== null);

  // (Re)tessellate each cad:body whose brep revision changed — keyed on
  // the artifact ref so stale-mesh reuse is impossible.
  const want = cadObjs
    .map((o) => {
      const brep = o.components["cad:shape"]?.data?.brep ?? "";
      const stale = !!o.components["cad:shape"]?.data?.stale;
      return { id: o.id, name: o.name, brep, stale };
    })
    .filter((b) => b.brep);
  const wantKey = want.map((b) => `${b.id}@${b.brep}`).join("|");

  useEffect(() => {
    let live = true;
    (async () => {
      const next = new Map<string, Body>();
      for (const b of want) {
        try {
          const m = await api.cadMesh(b.id);
          next.set(b.id, { id: b.id, name: b.name, mesh: m });
        } catch (e) {
          if (live) setMeshErr(`mesh ${b.name}: ${String(e)}`);
        }
      }
      if (live) {
        setBodies(next);
        setMeshErr("");
      }
    })();
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wantKey]);

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

    // ground grid + axes
    ctx.strokeStyle = "#21262d";
    ctx.lineWidth = 1;
    for (let i = -6; i <= 6; i++) {
      line(ctx, proj([i, 0, -6]), proj([i, 0, 6]));
      line(ctx, proj([-6, 0, i]), proj([6, 0, i]));
    }
    ctx.strokeStyle = "#3d4459";
    line(ctx, proj([0, 0, 0]), proj([3, 0, 0]));
    line(ctx, proj([0, 0, 0]), proj([0, 3, 0]));
    line(ctx, proj([0, 0, 0]), proj([0, 0, 3]));

    hit.current = [];

    // ---- real BRep meshes (painter-sorted triangles, lambert) ------
    interface Tri {
      p: [number, number, number][];
      depth: number;
      color: string;
      id: string;
    }
    const tris: Tri[] = [];
    const light: V3 = norm([0.4, 0.85, 0.55]); // view-space light
    for (const b of bodies.values()) {
      const { positions, normals, indices } = b.mesh.mesh;
      const sel = b.id === selectedId;
      const base = b.mesh.stale ? [0xd2, 0x8a, 0x3f] : [0x58, 0xa6, 0xff];
      const verts = positions.map(zu).map(view);
      const norms = normals.map(zu);
      for (let i = 0; i + 2 < indices.length; i += 3) {
        const [a, b2, c] = [indices[i], indices[i + 1], indices[i + 2]];
        const depth = (verts[a][2] + verts[b2][2] + verts[c][2]) / 3;
        if (depth <= 0.2) continue;
        // face normal = mean of corner normals (already normalized)
        const n = norm([
          (norms[a][0] + norms[b2][0] + norms[c][0]) / 3,
          (norms[a][1] + norms[b2][1] + norms[c][1]) / 3,
          (norms[a][2] + norms[b2][2] + norms[c][2]) / 3,
        ]);
        const lam = 0.25 + 0.75 * Math.abs(n[0] * light[0] + n[1] * light[1] + n[2] * light[2]);
        const col = sel
          ? `rgb(255,211,61)`
          : `rgb(${Math.round(base[0] * lam)},${Math.round(base[1] * lam)},${Math.round(base[2] * lam)})`;
        tris.push({
          p: [proj2(verts[a]), proj2(verts[b2]), proj2(verts[c])],
          depth,
          color: col,
          id: b.id,
        });
      }
      // label + hit region at the body's bbox center
      const bb = b.mesh.measures?.bbox;
      if (bb) {
        const c = proj(zu([
          (bb.min_mm[0] + bb.max_mm[0]) / 2,
          (bb.min_mm[1] + bb.max_mm[1]) / 2,
          (bb.min_mm[2] + bb.max_mm[2]) / 2,
        ]));
        const ext = Math.max(
          ...bb.max_mm.map((v: number, i: number) => v - bb.min_mm[i]),
        );
        hit.current.push({ id: b.id, x: c[0], y: c[1], r: Math.max(ext * f / Math.max(c[2], 0.5) * 0.4, 10) });
        ctx.fillStyle = sel ? "#ffd33d" : b.mesh.stale ? "#d28a3f" : "#8b949e";
        ctx.font = "11px sans-serif";
        ctx.fillText(`${b.name}${b.mesh.stale ? " ⟳stale" : ""}`, c[0] + 8, c[1]);
      }
    }
    tris.sort((a, b) => b.depth - a.depth);
    for (const t of tris) {
      ctx.beginPath();
      ctx.moveTo(t.p[0][0], t.p[0][1]);
      ctx.lineTo(t.p[1][0], t.p[1][1]);
      ctx.lineTo(t.p[2][0], t.p[2][1]);
      ctx.closePath();
      ctx.fillStyle = t.color;
      ctx.fill();
    }

    function proj2(v: V3): [number, number, number] {
      const s = f / Math.max(v[2], 0.5);
      return [w / 2 + v[0] * s, h / 2 - v[1] * s, v[2]];
    }

    // ---- geom:* primitive proxies (unchanged) ----------------------
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
        ring(dims[0] / 2);
        ring(dims[0] / 2 - dims[1] / 2);
      } else {
        const [sx2, sy2, sz2] = [dims[0] / 2, dims[1] / 2, dims[2] / 2];
        const hh = p.kind === "plane" ? 0.02 : sy2;
        const corners: V3[] = [
          [-sx2, -hh, -sz2], [sx2, -hh, -sz2], [sx2, -hh, sz2], [-sx2, -hh, sz2],
          [-sx2, hh, -sz2], [sx2, hh, -sz2], [sx2, hh, sz2], [-sx2, hh, sz2],
        ].map(([x, y, z]) => [x + p.pos[0], y + p.pos[1], z + p.pos[2]] as V3);
        const pc = corners.map(proj);
        ctx.fillStyle = shade(p.color, sel);
        ctx.strokeStyle = sel ? "#ffd33d" : p.color;
        ctx.lineWidth = sel ? 2 : 1;
        poly(ctx, [pc[0], pc[1], pc[2], pc[3]]);
        poly(ctx, [pc[4], pc[5], pc[6], pc[7]]);
        ctx.beginPath();
        for (const [a, b] of [[0, 4], [1, 5], [2, 6], [3, 7]]) {
          ctx.moveTo(pc[a][0], pc[a][1]);
          ctx.lineTo(pc[b][0], pc[b][1]);
        }
        ctx.stroke();
      }
      ctx.fillStyle = sel ? "#ffd33d" : "#8b949e";
      ctx.font = "11px sans-serif";
      ctx.fillText(p.name, cx + rad + 4, cy2);
      hit.current.push({ id: p.id, x: cx, y: cy2, r: Math.max(rad, 8) });
    }

    if (prims.length === 0 && bodies.size === 0) {
      ctx.fillStyle = "#484f58";
      ctx.font = "13px sans-serif";
      ctx.fillText("No geometry yet — create one with ⌘ Command or the agent panel.", 20, 30);
    }
    if (bodies.size > 0) {
      const trisN = tris.length;
      ctx.fillStyle = "#484f58";
      ctx.font = "10px sans-serif";
      ctx.fillText(`${bodies.size} cad:body · ${trisN} tris · mm`, 8, h - 8);
    }
    if (meshErr) {
      ctx.fillStyle = "#f85149";
      ctx.font = "11px sans-serif";
      ctx.fillText(meshErr, 20, 30);
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

function norm(v: V3): V3 {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
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
