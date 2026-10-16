---
name: fieldcraft-meshing
description: Agentic hex meshing (AHM) in FieldCraft — read the part, plan partitions, and build a structured all-hex (or hex-dominant) mesh an analyst would accept. Use before any mesh.* command on a part that matters, and whenever a mesh comes back with fans, slivers or a low scaled Jacobian.
---

# Agentic hex meshing (AHM)

You mesh like an experienced analyst: you look first, plan the blocks, and only then call a mesher.
A mesh is judged by its worst elements, not its count. A fast mesh with fans, slivers or collapsed
corners is a failed mesh, even when every element is "valid".

## 0. Start from the provenance graph (SolidCraft designs)

A `.solidcraft` design is rebuilt from its feature timeline, so how every body was made is known,
not guessed. Read it before you plan anything:
1. `geometry.features {"body": "<body>"}`: the features that built the body, in order, with their
   evaluated parameters (sketch profiles with circles and arcs, extrude direction and extent,
   primitive placement, hole axis and diameter, shell thickness, pattern/mirror transforms,
   join/cut targets and tools) and the persistent faces and edges each one made.
2. `mesh.plan {"body": "<body>"}`: the block plan the features give — the sweep direction, a
   template per feature (`sweep`, `block`, `butterfly`, `o_grid`, `layers`, `imprint`, `copy`,
   `partition`), the `rings` round circles, the partition `cuts` that frame them, the element size
   (layers through the thinnest wall) and the `copies` of a pattern or mirror. Read the
   `unmapped` list: revolves, sweeps and lofts have no template yet, and a feature not
   prismatic along the sweep needs projection blocking (#399). Fillets and chamfers map to a
   `blend` layer.
3. Edit the plan if the size is wrong (apply the edited object), then
   `mesh.plan.apply {"plan": "Plan-<body>"}`. When the plan has rings or blends and the body
   fits, its `mesher` is `project`: one block topology over the whole body (a grid on the
   body's planar faces, an O-grid square-inside-square round every circle, concentric ones
   nested: a boss and its hole, frames centred on the hole; a screw hole in a rounded corner
   takes the corner's fillet into its ring; a blend layer along fillets, chamfers and shell walls
   at their stored depth) projected and smoothed — no partitions, one conforming mesh, one division
   count per sheet of blocks. Read `layout` and `blend_layers`; the plan's `notes` say why a
   body did not fit (a plane at an angle, a curved face that is not a ring or a blend, two
   circles in one frame), and then the frame cuts and swept cells are used instead (`pending`
   lists the rings left boxed). Check `all_hex`, `conforming`, `min_scaled_jacobian`.
   Known gap: a shelled box whose fillet radius exceeds the wall (the enclosure-base pattern)
   can still leave a twisted element at the outer fillet's bottom corner.
4. Then verify as in §4. When the plan does not fit (unmapped primary features, a failed cell),
   fall back to the manual process below, keeping the plan's cuts that work.

Shapes without a timeline (STEP, 3MF) start at §1.

## 1. Look before you mesh (always)

Before the first mesh command:
1. `inspect_model` / `geometry.bodies`: bodies, volumes, bounding boxes, faces, holes, fillets.
2. Take screenshots from the principal views and an iso view. Name, in words, what the part is.
3. **Symmetry.** Test it geometrically (mirror the bounding box and the key faces). If the part (and
   the loads you will apply) are symmetric, mesh one half or quarter only and mirror the mesh
   (`mesh.mirror`). Never mesh a symmetric part whole.
4. **Primary vs secondary features.**
   - Primary: what carries the physics and sets the topology — the main body, walls, bosses,
     the load path, thin directions, the direction a section is swept along.
   - Secondary: small fillets, chamfers, logos, small holes far from the region of interest.
   Start the block structure from the primary features. Secondary features are fitted into it
   (or defeatured), never allowed to drive it.
5. **Where to split.** Before meshing, list the cuts: planes or curved surfaces that turn every
   region into a six-sided block or a sweepable prism. Say why each cut is there.

## 2. Plan the blocks (the rules)

- **Never make sharp corners.** No element angle near 0° or 180°. Where a block would have a
  corner angle far from 90°, cut differently or add a block (an O-grid) so every corner in the mesh
  is close to 90°.
- **Partitions must never create skewed, tapered or high-aspect-ratio hex.** Check every cell you
  create: if it is a wedge, a sliver or a long thin strip relative to the element size, the cut is
  wrong. Move or remove it.
- **Butterfly (O-grid) around curved parts.** Around a hole, a boss, a cylinder or a fillet, cut a
  square block around it and an O-grid ring between the square and the curve, so element lines run
  radially and the corners of the square sit at 45° on the curve. Never let a mesh fan into a hole
  or pinch at a tangency.
- **Split and merge to make it square.** When a region is far from a square (an L, a long strip
  with a bump), split it into pieces and merge pieces again so each final cell is closer to a cube.
  Aim for cells with aspect ratio near 1 at the target size.
- **Curved splits are allowed.** A cut does not have to be a plane: use a cylindrical or offset
  surface (a ring around a boss, an offset of a fillet) when a plane would create slivers.
- **Never cut along an existing face** (a plane coincident with a face within tolerance leaves
  micrometre slivers, #384) and never through the axis of a small hole (degenerate tessellation at the
  hole edge). Offset the cut to just outside the feature instead.
- **Keep cuts consistent across touching bodies** so their meshes can match, or tie them
  deliberately (`interaction.tie`; linear faces only today).

## 3. Mesh

- Mesh each cell with the structured method that fits (`mesh.hex.generate` extrude/sweep/revolve/
  mapped; `mesh.cells` for partitioned parts; `mesh.hex.blocks` when every cell is a block).
- Use edge seeds (`mesh.seed.edge`) so counts match across shared edges and so there are enough
  elements through thin walls (at least 2–3 through a wall for bending, 1 for pure conduction only).
- Tet (`tet10` preferred for stress) only for cells that are not worth blocking, and only where
  they are far from the region of interest. Before falling back to tet on a region that ends on a
  curved face, try extrude-to-surface (§3b).

## 3b. Extrude up to a surface — try it before giving up on a region

A region that is one section carried to a curved or tilted end (a boss or rib meeting a curved
wall, an air column under a domed lid, a fillet region capped by a curved face) is not sweepable
between parallel faces, but `mesh.hex.extrude` meshes it:

- `source`: the section's face. If a neighbour (or the cell below) is already meshed there, its
  quads are reused and the two meshes share nodes; else a planar face is meshed here at `size`.
  A 2D mesh works too (`source_set`).
- `to_surface`: the end faces; `sides`: the region's side faces (give them whenever they are not
  straight along the path, so every layer follows them).
- `direction` for a straight region, else `normal`; `layers` (or `size`) and `bias` (< 1 grades
  toward the target, e.g. for a boundary layer under a lid).
- Check the reply: `max_target_gap` ≈ 0, `min_scaled_jacobian` ≥ 0.5, `section.shared_nodes`
  equal to the section's nodes when it should conform. An error names the node whose path
  misses the target or the element that would invert: shorten the region (cut it where the end
  turns steep) rather than falling back to tets.

## 3a. Projection blocking

When partitioning the real geometry into sweepable cells is awkward (round parts, branches,
handles, curved caps), lay a block topology over the part and project it (`mesh.hex.project`):

- **Sphere or ball-like region**: `"template": "sphere"` (inner cube + six blocks). Never a single
  block on a sphere.
- **Disc, rod, tube, any round section**: `"template": "butterfly"` (centre square + four petals)
  along the axis, with `levels` at every change along it. A plain grid block projected onto a
  cylinder flattens its corners to 180° — the command refuses it (`min_quality`).
- **Holes, bosses, floors of cups**: O-grid the block (`{"op": "o_grid", "block", "open", "inset"}`);
  open faces let the inner block reach that face. A cup floor needs its own O-grid inside the
  wall's butterfly (`open: [1, 2]` on the centre block), the cavity is then `remove`d.
- **Side branch**: O-grid the block under the branch's footprint open toward it and extrude the
  open face (`"extrude": {"offset", "layers"}`, faces `all` = round branch). The junction edges go
  onto the curve where the two surfaces meet automatically.
- **Square stubs, handles, ribs**: O-grid open toward the stub, extrude `inner`, chain `extrude`
  ops, and close loops with `connect`. Inner corners of a stretched topology follow the
  projection, but keep the initial blocks roughly where the geometry is (offsets measured from
  the block faces, not from the surface).
- **Footprint margins**: a stub's footprint must sit in the O-gridded block face with roughly even
  margins all round. A 6 mm footprint in a 30 mm wide petal face with 2 mm above and below leaves
  ring corners near 170° that twist. Split first (before the O-grid),
  `{"op": "split", "block", "direction": "y", "at": [0.36, 0.64]}`, so the side margins match the
  levels' margins; the split runs through the whole sheet of blocks, so preview again for the
  new indices.
- A twisted-element error names the block, cell and the block vertex (junction) with the blocks
  meeting there: fix the topology at that junction (split, move a level, change the inset).
- Use `"preview": true` to list block indices before meshing; faces can be given as `"+x"` … `"-z"`.
- Targets default to the smooth group of B-rep faces each outer block face faces; set them with
  `faces: [{block, face, to}]` when a block face is near two surfaces.
- Read the reply: `initial_min_scaled_jacobian` (transfinite grid) vs `min_scaled_jacobian`
  (after elliptic smoothing), `max_gap` (should be ~0). An error names the worst block and where.

## 4. Verify (every time, before solving)

1. `mesh.quality`: all valid; scaled Jacobian min ≥ 0.3 (≥ 0.5 in the region of interest);
   edge ratio max ≤ 10; no element below these is acceptable without a stated reason.
   **Isolated spikes** (a few bad elements in an otherwise good mesh) are fixed last, with
   `mesh.node.relax {"around_element": <worst>}` (or `center` + `radius`): nodes there relax, surface
   nodes sliding on their faces, nothing inverting. It is a final touch, never a substitute for a
   better block plan: many bad elements mean the cuts are wrong (§2). Pass `"reapply": true` (or
   run `mesh.node.relax.apply {field}`) so the relaxation follows a later remesh.
2. Look at it: screenshots of each body and a section. Fans, pinches, wedges, slivers → back to §2.
3. Volume: `mesh.quality` volume equals the geometry volume (a mismatch means orphan or missing
   cells, #385).
4. `mesh.check.duplicates`: no cracks or free edges inside a body.
5. After solving, refine once and compare the quantity of interest (convergence).

## 5. Report

State: symmetry used, primary features, every cut and why, method per cell, element counts,
quality numbers, and anything you defeatured. If the plan failed, say which rule failed and the
next cut you will try — don't silently fall back to a worse mesh.
