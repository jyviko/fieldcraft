# Capability checklist (reference: Abaqus/CAE)

What an analyst does in a full CAE pre-processor, grouped by milestone, with ModelCraft's status.
Scope comes from the public documentation of Abaqus/CAE and public analyst discussion,
described in our own words; nothing was taken from the product itself (AGENTS.md: no commercial
products are run, no documentation text is copied). Status: **done**, **started**, **planned**.
**HEX** marks items centred on hexahedral meshing, the core goal.

## Where we aim to be better

| # | Opportunity | Why analysts want it | ModelCraft approach |
|---|---|---|---|
| 1 | Hex meshes without manual partitioning | Partitioning a part into sweepable cells by hand is the most cited chore | Sweep and mapped meshing with automatic decomposition that proposes SolidCraft splits (M5); octree all-hex for anything else (M6) |
| 2 | Undo, redo, replay, crash recovery | No general undo; replay files are hard to reuse; corrupted model files | Every action is a journalled command with rollback |
| 3 | Sets that survive changes | Sets and loads break when geometry regenerates or is remeshed | Sets bind to SolidCraft's persistent face, edge and body names and resolve after meshing (M3) |
| 4 | An API that is not a bundled scripting runtime | Version breakage, slow scripts, geometry picked by position | JSON commands over CLI, control channel and MCP |
| 5 | Located diagnostics | Generic error messages | Errors name the element, face or keyword; `mesh.quality` lists failing elements |
| 6 | Lossless deck round trip | Unsupported keywords dropped on import | Deck readers keep unknown keywords and write them back (M9) |
| 7 | Safe, diffable model files | Binary model databases that corrupt or lock to a version | Text `.modelcraft` model, atomic saves, autosave (M3) |
| 8 | Geometry checked before meshing | Slivers and tiny faces fail late | Surface preparation reports defects with their location (M1) |
| 9 | No size caps, any solver | Node limits in learning editions; one solver per tool | Free, open source, three deck formats |
| 10 | Clear step history | Loads and BCs propagate between steps implicitly | An explicit step × load/BC table (M3) |

## Checklist

| Area | Capability | Milestone | HEX | Status |
|---|---|---|---|---|
| Geometry | Open SolidCraft designs, bodies, persistent face names, assemblies placed | M1 | | done |
| Geometry | Face-tagged mesh import (3MF) | M1 | | done |
| Geometry | Weld, closed/manifold/orientation and self-intersection checks, feature edges | M1 | yes | started |
| Geometry | Partitions and datums as SolidCraft features (split body/face, construction planes) | M5 | yes | planned |
| Geometry | Midsurface extraction for shells | M8 | | planned |
| Mesh | Size field: global, per face and edge seeds, bias, curvature and proximity refinement | M2 | yes | planned |
| Mesh | Free tet meshing (tet4, tet10 with mid-side nodes on the geometry) | M2 | | started |
| Mesh | Quality metrics, certified validity, verify per set | M1–M2 | yes | done (metrics, gate) |
| Mesh | 2D quad meshing of faces | M5 | yes | planned |
| Mesh | Mapped hex of six-sided regions | M5 | yes | planned |
| Mesh | Sweep hex (extrusions, revolutions) | M5 | yes | planned |
| Mesh | Automatic decomposition into sweepable and mappable regions | M5 | yes | planned |
| Mesh | Conforming interfaces, tet–hex transition with pyramids | M5 | yes | planned |
| Mesh | Octree/grid-dual all-hex for any part | M6 | yes | planned |
| Mesh | Hex-dominant by recombination; polycube | M6 | yes | planned |
| Mesh | Smoothing, untangling, boundary padding, layers through thin walls | M6 | yes | planned |
| Mesh | Mesh editing (move, merge, split, collapse, remesh region), imported orphan meshes | M6 | yes | started (node/element add/remove) |
| Model | Sets and surfaces bound to geometry names | M3 | yes | done |
| Model | Materials: elastic (isotropic, orthotropic), density, unit-system check | M3 | | done |
| Model | Materials: plasticity, hyperelasticity, damage, thermal | M8 | | planned |
| Model | Sections: solid, shell, beam profiles; assignment by set | M3 | | done |
| Model | Composite layups, material orientations, continuum shells | M8 | yes | planned |
| Model | Element formulation per region (integration, incompatible modes, hybrid) mapped per solver | M3 | yes | done (model; per-solver mapping with the deck writers) |
| Model | Boundary conditions: fixed, displacement, symmetry | M3 | | done |
| Model | Loads: force and moment, pressure, gravity, body force | M3 | | done |
| Model | Reference points, kinematic and distributing coupling | M3 | | done |
| Model | Steps: linear static, frequency; output presets | M3 | | done |
| Model | Steps: nonlinear static, explicit dynamics, heat transfer; amplitudes | M7–M8 | | planned |
| Interaction | Tie between non-matching meshes | M7 | | planned |
| Interaction | Contact (general and pairs), friction, automatic pair detection | M7 | | planned |
| Interaction | Bolt pretension, springs, point masses, connectors | M7 | | planned |
| Decks | Abaqus input writer | M3 | | planned |
| Decks | Nastran bulk data writer (SOL 101, 103) | M3 | | planned |
| Decks | LS-DYNA keyword writer (implicit; explicit in M8) | M3 | | planned |
| Decks | Readers keeping unknown keywords | M9 | | planned |
| Solve | Own linear static and modal solver, verified against analytic solutions | M4 | yes | planned |
| Results | Deformed shape, contours, mode shapes, probes, reaction sums | M4 | | planned |
| Results | XY and path plots, cuts, display groups, free-body forces, animation | M9 | | planned |
| Results | Readers for public result formats (Nastran OP2, LS-DYNA d3plot, Abaqus `.fil`) | M9 | | planned |
| Results | Mesh convergence studies | M9 | | planned |
| App | Desktop viewport, model tree, commands, control channel | continuous | | done |
| App | Undo/redo on the command journal, autosave, `.modelcraft` file | M3 | | planned |
| App | MCP server, browser build | continuous | | planned |

## Deck notes for the writers

- Node order: the model stores Abaqus/VTK order. Nastran orders the mid-side nodes of 20-node hexes
  and 15-node wedges differently; LS-DYNA takes tets and wedges as degenerate hexes and triangles
  as degenerate quads. Every mapping is tested by the sign of the element Jacobian after writing.
- Each format's field widths, continuation rules, number forms and label limits are respected by
  the writer and checked by golden-file tests on our own examples.
