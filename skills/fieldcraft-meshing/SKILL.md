---
name: fieldcraft-meshing
description: Agentic hex meshing (AHM) in FieldCraft — look at the part, use its symmetry, plan the blocks, mesh, mark and fix the bad elements, and show sections an analyst would accept. Use before any mesh.* command on a part that matters, and whenever a mesh comes back with fans, slivers or a low scaled Jacobian.
---

# Agentic hex meshing (AHM)

Mesh like an analyst: look first, plan the blocks, then call a mesher. A mesh is judged by its
worst elements, not its count. A fast mesh with fans, slivers or collapsed corners is a failed
mesh, even when every element is valid.

The loop: **look → symmetry → plan → inspect the plan → apply → quality + mark → relax → section
pictures → report.** Never skip a step because the last mesh looked fine.

## 1. Look before you mesh

1. `inspect_model` / `geometry.bodies`: bodies, volumes, bounding boxes, faces, holes, fillets.
   For a `.solidcraft` design also `geometry.features {"body"}`: how each body was built
   (profiles, extrude directions, holes, shells, fillets, patterns, mirrors).
2. `geometry.check {"body", "size"}` at the size you intend: short edges, small and sliver faces,
   near faces (gaps, overlaps, walls thinner than the gap). Each finding names its faces and a
   point `at`, and is marked in pictures. Defeature or resize before meshing; never mesh over a
   finding you have not explained.
3. Take pictures. Headless: `fieldcraft-cli snapshot script.json --out a.png` with
   `--view iso|front|top|…` or `--camera '{"yaw":-45,"pitch":30,"target":[x,y,z],"fit":true}'`,
   and `--section x|y|z[=VALUE]`. Over MCP: `screenshot {view | camera: {yaw, pitch, target,
   distance | fit}, section}`. Aim `target` at the feature you are judging (a boss, a corner), not
   the model's centre. Read every PNG. Name, in words, what the part is.
4. Primary vs secondary features. Primary (walls, bosses, the load path, thin directions, the
   sweep direction) set the block topology. Secondary (small fillets, chamfers, logos, small holes
   far from the region of interest) fit into it or are defeatured; they never drive it.

## 2. Symmetry first

Test symmetry geometrically (mirror the bounding box and the key faces). If the part and its
loads are symmetric, mesh one half or quarter and mirror the mesh: `mesh.mirror {"plane": <datum
plane> | {point, normal}}`. Nodes on the plane are merged, so the result is conforming. Never mesh
a symmetric part whole. If the halves come out unequal (one corner good, its mirror image bad),
keep the good half and mirror it.

`mesh.plan` refuses a partitioned body and its cells today (#473): for a half, mesh the cell with
`mesh.cells` or `mesh.hex.project`, then mirror.

## 3. Plan, then inspect the plan

1. `mesh.plan {"body", "size"?, "layers"?}`: the block plan from the features. Nothing is meshed.
2. Read the plan before applying it:
   - `mesher`: `project` (one block topology over the body: O-grid rings round circles, blend
     layers along fillets, chamfers and shell walls; for a section at an angle to the axes, such
     as a crank arm, an annulus round each boss's hole and a carved grid with a pillow layer to
     the outline) is the target; `cells`/`sweep` is the fallback.
   - `rings`: centre, radius, frame half-side. Frames should be near square with even margins.
   - `cuts`: each with its feature and reason.
   - `notes`: why the body did not fit the projection layout.
   - `pending` (rings left boxed) and `unmapped` (features with no template: revolves, sweeps,
     lofts).
   - `size` and `layers` against the thinnest wall (§5).
   Fix what is wrong by editing the plan object and applying the edit.
3. `mesh.plan.apply {"plan": "Plan-<body>" | edited object}`. Read `skipped_cuts`, `all_hex`,
   `conforming`, `min_scaled_jacobian`, `layout`, `blend_layers`. A skipped cut is a defect to
   explain.
4. When the plan does not fit, mesh regions by hand and keep the plan's cuts that work:
   `geometry.partition` + `mesh.cells`; `mesh.hex.generate` / `mesh.hex.sweep` / `mesh.hex.mapped`;
   `mesh.hex.extrude` for a section carried to a curved or tilted end (`source`, `to_surface`,
   `sides`); `mesh.hex.project` for a block topology projected on the part (`template` `sphere` or
   `butterfly`, ops `o_grid`, `split`, `extrude`, `connect`; `preview: true` lists block indices).
   Tet (`tet10` for stress) only for cells not worth blocking, far from the region of interest.

Shapes without a timeline (STEP, 3MF) skip `mesh.plan` and go to step 4.

## 4. Block rules

- **No sharp corners.** Every element corner near 90°. Where a block corner is far from 90°, cut
  differently or add an O-grid.
- **No skewed, tapered or long thin cells.** Check every cell a cut makes; a wedge, sliver or
  strip means the cut is wrong.
- **O-grid round every hole, boss, cylinder and fillet**: a square round the curve and a ring
  between, element lines radial, square corners at 45° on the curve. Never fan into a hole or
  pinch at a tangency.
- **Never cut on an existing face.** A coincident cut leaves slivers, and one the kernel can only
  make by nudging off the plane is refused (`geometry.partition`). Offset the cut one element out
  of the feature. Never cut through the axis of a small hole.
- **Parity at corners.** Division counts must close round every block loop. Where a corner comes
  out bad and its mirror image is good, change the count by one on the edges leading into it
  (`mesh.seed.edge`) first.
- **Pattern a good corner.** When identical features (four corner bosses) mesh unequally, the
  good one is the template: mirror it onto the others (`mesh.mirror`) instead of accepting the
  worst.
- **Accept a wedge column** where the layout forces one node to absorb a corner: one column of
  `wedge6` is better than a column of skewed hex. State it in the report.
- **Keep cuts consistent across touching bodies** so the meshes match, or tie them
  (`interaction.tie`).

## 5. Size and walls

- Through a thin wall: at least 2 elements for bending (3 for stress in the wall), 1 only for
  pure conduction. Set `layers` in `mesh.plan`, or `mesh.seed.edge` across the wall.
- Matching counts on shared edges: `mesh.seed.edge`.
- Size from the physics: finer at stress gradients, fillets and contact; in explicit, from the
  wave length.

## 6. Quality, mark, relax

1. `mesh.quality`: all valid; volume equal to the geometry's (a mismatch means missing or orphan
   cells).
2. `mesh.quality.mark {"limits": {"scaled_jacobian": 0.3, "edge_ratio": 10}}`: failing elements
   become the set `quality_marked`, drawn red in pictures, each with `label`, `at`, `failed`.
   Acceptance limits:
   - scaled Jacobian ≥ 0.3 everywhere, ≥ 0.5 in the region of interest;
   - edge ratio and aspect ratio ≤ 10;
   - face angles 15°–165°, dihedrals 10°–165° (the defaults).
3. Picture the marked elements (camera `target` at their `at`, a `section` through them).
   Clustered marks mean the blocks are wrong: back to §3/§4. Isolated spikes only:
   `mesh.node.relax {"around_element": <label>}` for each marked element (by CAD face once #471
   lands; sets are for the analysis steps, not for relaxing). Surface nodes
   slide on their faces; nothing inverts. Add `"reapply": true` so it follows a remesh. Relaxing
   is the final touch, never a substitute for a better plan.
4. Mark again until nothing is marked, or every remaining mark has a stated reason.
5. `mesh.check.duplicates`: no cracks or free edges inside a body.
6. Section pictures through every primary feature: fans, pinches, wedges, slivers or zigzag lines
   → back to §3.
7. After solving, refine once and compare the quantity of interest (convergence).

## 7. Report

State: symmetry used, primary features, the plan's mesher, every cut and skipped cut with its
reason, method per region, element counts, min scaled Jacobian, max edge ratio, marked count, what
was relaxed, any wedge column, anything defeatured, and the section pictures. If the plan failed,
name the rule that failed and the next thing you will try. Never silently fall back to a worse
mesh.
