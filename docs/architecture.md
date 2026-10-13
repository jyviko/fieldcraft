# Architecture

ModelCraft turns geometry into finite element models, writes them for the solvers engineers
already use, and solves linear problems itself.

```
SolidCraft B-rep ──tessellate──▶ boundary surface ──▶ volume mesh ──▶ FE model ──▶ Abaqus .inp
 (solidcraft-kernel)           (welded, watertight,   tet (Delaunay)   materials,     Nastran .bdf
                                face-tagged)          hex (core goal)  sections,      LS-DYNA .k
                                                                       BCs, loads,  ──▶ own linear
                                                                       steps            solver
```

## Crates and layers

`cargo xtask layers` enforces the table in `xtask/src/layers.rs`: a crate may depend only on
crates in strictly lower layers.

| Layer | Crate | Role | Status |
|---|---|---|---|
| L0 | `modelcraft-model` | Solver-neutral FE model: nodes and elements by label, their geometry tags, sets, materials, sections, couplings, BCs, loads and steps, the model check and the resolved view the writers read. Knows no file format and no geometry type. | started |
| L1 | `modelcraft-geometry` | The only door to SolidCraft: opens `.solidcraft` designs (rebuilt from their timeline, bodies with persistent face names), B-rep bodies, measures, tessellation into a face-tagged `SurfaceMesh`; face-tagged 3MF files read into the same type without the kernel (docs/formats/tagged-mesh.md). No SolidCraft type in its API. | started |
| L1 | `modelcraft-mesh` | Surface preparation (weld, watertightness, feature edges), robust predicates, Delaunay tetrahedralization and refinement, Voronoi-based optimization, hex meshing, quality metrics. | started |
| L1 | `modelcraft-solver` | Element formulations, assembly, sparse linear static and modal solution. | planned |
| L1 | `modelcraft-decks` | Abaqus input, Nastran bulk data and LS-DYNA keyword readers and writers, to and from `model`. | planned |
| L1 | `modelcraft-render` | Display data without a GPU dependency: the visible faces, element edges and nodes of a model, the orbit camera, a CPU rasterizer for headless snapshots; contour maps later. | started |
| L2 | `modelcraft-doc` | The document: geometry, mesh, model, results, history. | planned |
| L3 | `modelcraft-io` | `.modelcraft` files, result files. | planned |
| L4 | `modelcraft-engine` | Commands (`CommandSpec`), `Session::execute` with rollback and the panic guard, scripts. | started |
| L5 | `modelcraft-ui-egui`, `modelcraft-mcp` | Desktop UI (egui + wgpu viewport, model tree, commands, status bar) and control channel requests (docs/control-protocol.md); MCP server. | ui started |
| testkit | `modelcraft-testkit` | Test tooling, a dev-dependency only: the meshing test corpus (`tests/corpus/*/recipe.json`, analytic shapes) and its runner (`cargo xtask corpus`). | started |
| apps | `modelcraft`, `modelcraft-cli`, `modelcraft-web` | Desktop, command line, browser. | desktop, cli started |

Rules beyond the layer order:
- Nothing below L5 uses egui, eframe, winit, wgpu or rfd.
- Only `modelcraft-geometry` uses `solidcraft-*`, pinned by rev in the workspace `Cargo.toml`.
  SolidCraft's `[patch.crates-io]` (its fixed truck crates) does not reach dependents, so the
  workspace repeats it from the same rev; bump both together. The same goes for SolidCraft's
  patched `egui-wgpu` (MSAA clamped to what the device supports, 1x on OpenGL adapters, so the
  desktop app's 4x never fails pipeline validation at start).
- Everything below the apps builds for `wasm32-unknown-unknown`.
- `modelcraft-geometry`'s non-default `scripts` feature builds designs from SolidCraft command
  scripts (`script_design`, through `solidcraft-engine`) for the test corpus; the library opens
  designs without it.

## Meshing pipeline

1. **Boundary surface.** SolidCraft tessellates each body; every triangle carries its B-rep face,
   so feature edges and face-based loads and BCs survive meshing. Faces of an opened design also
   carry SolidCraft's persistent names (`F2:side:l4`, from the feature and sketch curve that made
   them), so a load on a named face follows it when the design is edited and opened again.
   Bodies of components are placed in the world through their occurrences (one body per
   placement), as SolidCraft displays and exports them.
2. **Surface preparation** (`modelcraft-mesh::surface`, `mesh.surface.check`). Weld the tessellation's per-face
   vertex copies (at twice SolidCraft's linear tolerance), check the surface is closed, manifold, consistently and
   outwardly oriented (voids inwards) and free of self-intersections (exact triangle–triangle tests), and find
   feature edges (face boundaries, sharp dihedral angles). Defects come back located by triangle, vertex and
   position.
3. **Tetrahedral meshing.** Bowyer–Watson Delaunay on Shewchuk's adaptive predicates (symbolic
   perturbation for degeneracies), boundary recovery by the robust constrained Delaunay method of
   Diazzi et al. 2023 (implicit Steiner points on indirect predicates, gift-wrapping fallback), so
   every boundary triangle and its B-rep face tag survives. Then constrained Delaunay refinement to a
   size field, ODT smoothing interleaved as in Tournois et al. 2009, and the Klingner–Shewchuk 2007
   improvement schedule against slivers.
4. **Hexahedral meshing (the core).** Octree/grid dual all-hex first, the one family that meshes
   any input validly (Livesu et al. 2021 dual schemes, Pitzalis et al. 2021 refinement, Gao et al.
   2019 feature projection and padding, Garanzha et al. 2021 untangling). Then hex-dominant by tet
   recombination (Pellerin et al. 2018), then polycube (DualCube labelling) for blocky parts;
   frame-field/integer-grid-map methods stay research. Validity uses a certified trilinear Jacobian
   check (Marschner et al. 2020), not corner Jacobians alone.
5. **Structured hex and face meshing.** Faces get all-quad meshes on shared, prescribed edge
   nodes (`modelcraft-mesh::quad`): transfinite interpolation for four-sided faces (Gordon and
   Hall 1973), and for free faces a half-resolution constrained Delaunay triangulation,
   recombination (Lee and Lo 1994) and one uniform split that lands on the boundary nodes, then
   smoothing and untangling; every quad is checked with exact predicates. One `SizeField` trait
   (`modelcraft-mesh::size`) sets interior sizes and edge divisions, with even counts per loop.
   Mapped and swept hex build on these face meshes.
6. **Quality.** Scaled Jacobian, aspect ratio, dihedral angles; every generated mesh is checked
   for positive Jacobians and conserved volume before it reaches the model.

## Model definition

`modelcraft-model` holds the analysis model in no solver's vocabulary (the writers map it to
element names, cards and keywords):

- **Geometry tags.** Geometry is referred to by name only: a body and, inside it, the persistent
  name of a face, edge or vertex (`{"body": "Body1", "face": "F2:top"}`). A mesher tags what it
  makes: each element with the body (solids), face (shells) or edge (beams) it fills, each
  boundary element face with its geometry face, element edges with geometry edges, nodes with
  vertices (`Model::geometry_tag`, `tag_element`, `tag_element_face`, …; `mesh.tag` by hand).
  Replacing or removing an element drops its tags; a remesh clears them and writes new ones.
- **Sets** are defined by geometry names, explicit labels or both, and resolve against the
  current mesh into sorted nodes, elements and element faces. A face name also selects the pieces
  a kernel splits it into (`name#0`, `name#1`). Because the definition names geometry, a remesh,
  or an edit of the design followed by a remesh, keeps every set, section and load in place.
- **Element faces** are numbered per kind (`ElementKind::faces`): solids' faces are oriented with
  the right-hand normal pointing out; shells have a positive and a negative side.
- **Definitions** (materials, sections with their element formulation, reference points,
  couplings, boundary conditions, loads, steps) refer to each other by name. A reference must
  exist when it is made, and a definition still referred to cannot be deleted; what depends on
  the mesh is checked when the model is resolved.
- **Steps** list the boundary conditions and loads active in them. Nothing propagates from one
  step to the next.
- **Units** are a label (`mm-t-s-N-MPa`, …); numbers are never converted, and the check warns
  when a modulus, density or gravity is implausible for the chosen system.
- **Check and resolve.** `Model::check` lists errors and warnings, each naming its item (a set
  member not on the mesh, an element with no section or in two, a moment on solid nodes, a
  missing density for a frequency step, …). `Model::resolve` returns what writers and the solver
  read besides the definitions: every set resolved, the section of every element, and the node
  label of every reference point (after the largest mesh node, in name order). It fails when the
  check finds an error.

## Commands

Every user-visible action is a `CommandSpec` in `crates/engine/src/cmd/<area>.rs` with a JSON
form. `Session::execute` clones the model, runs the command under `catch_unwind`, and restores
the model on an error or a panic; commands that changed the model are journalled, so the
journal replays as a script.

## Test corpus

Meshers are measured on our own parts (`docs/adr/0001-libraries.md`, test geometry). Each part is
a SolidCraft command script in `tests/corpus/<name>/recipe.json` (the file `solidcraft-cli run`
takes, plus a `corpus` object with its categories and, when it has one, its closed-form volume)
with a one-line `README.md`; analytic shapes (sphere, torus, cylinder, thick-walled pipe, plate
with a hole) are generated in code. `modelcraft-testkit` runs a recipe in SolidCraft's engine,
saves the design and opens it through `open_design`, the path a person's design takes, so no
geometry is stored. `cargo xtask corpus` (a release build of the testkit's `corpus` binary)
builds every part, tessellates it and runs each stage of `modelcraft_testkit::runner`, writing
docs/meshing/corpus.md. A mesher joins by implementing `runner::Stage` (its columns, and a run
over the part's tessellated surfaces) and adding itself to `runner::stages()`.
