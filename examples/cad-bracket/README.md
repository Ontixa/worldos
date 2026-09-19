# examples/cad-bracket

A parametric CAD bracket produced end-to-end by the CLI through a
structured command plan (`plan.json`) executed by `worldos batch` in
**one transaction** — the deterministic counterpart of an agent plan.

The part: a 60×40 mm plate with two Ø8 through holes, r1 fillet on all
plate edges, and a 6×40×16 stiffening rib unioned on top. The plan
finishes by editing the root stock thickness 8 → 10 mm and regenerating
the whole dependent chain in topological order.

Dependency depth:

```
plate-stock ─→ plate_h1 ─→ plate ─→ plate_f ─→ bracket
   hole1 ────↗     hole2 ──↗                  rib ──↗
```

Inspect it:

```powershell
worldos inspect bracket.worldos
worldos graph bracket.worldos                          # derived-from edges
worldos history bracket.worldos
worldos command bracket.worldos cad.measure '{"object":"bracket"}'
worldos undo bracket.worldos    # reverts the whole build transaction
worldos redo bracket.worldos    # replays it
worldos command bracket.worldos cad.export_step '{"object":"bracket"}'
```

Regenerate from scratch (Windows):

```powershell
./examples/cad-bracket/regenerate.ps1    # requires `worldos` on PATH
```

Or open `bracket.worldos` in the desktop app — the viewport renders the
real OCCT tessellation of `bracket`, and the inspector shows the
recipe params / stale state / dependency chain for every `cad:body`.

Everything goes through the same `Engine` as the RPC/MCP/SDK paths —
the plan file is exactly what an agent would emit.
