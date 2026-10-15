---
name: fieldcraft-meshing
description: Agentic hex meshing (AHM) in FieldCraft — read the part, plan partitions, and build a structured all-hex (or hex-dominant) mesh an analyst would accept. Use before any mesh.* command on a part that matters, and whenever a mesh comes back with fans, slivers or a low scaled Jacobian.
---

# Agentic hex meshing (AHM)

You mesh like an experienced analyst: you look first, plan the blocks, and only then call a mesher.
A mesh is judged by its worst elements, not its count. A fast mesh with fans, slivers or collapsed
corners is a failed mesh, even when every element is "valid".

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
  they are far from the region of interest.

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
