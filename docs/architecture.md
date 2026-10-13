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
| L1 | `modelcraft-fem` | Element kernels shared by every structural solver: the element table (tet4, tet10, hex8 full and incompatible modes, hex20 full and reduced), shape functions, quadrature, element stiffness, consistent and lumped mass, body and pressure loads, stresses and their extrapolation, isotropic and orthotropic elasticity. Pure functions, no solver loop. | started |
| L1 | `modelcraft-decks` | Abaqus input, Nastran bulk data and LS-DYNA keyword readers and writers, to and from `model`. Text out, no file system; `deck.write` saves it. | started |
| L1 | `modelcraft-render` | Display data without a GPU dependency: the visible faces, element edges and nodes of a model, the orbit camera, a CPU rasterizer for headless snapshots; contour maps later. | started |
| L2 | `modelcraft-solver` | Implicit solver: assembly from the resolved model, boundary conditions, sparse direct solution, linear static and modal analysis, stresses and reactions. The explicit (`modelcraft-explicit`) and lattice Boltzmann (`modelcraft-lbm`) solvers join this layer. | started |
| L3 | `modelcraft-fsi` | Fluid–structure coupling between the L2 solvers (later). | planned |
| L4 | `modelcraft-doc` | The document: geometry, mesh, model, results, history. | planned |
| L5 | `modelcraft-io` | `.modelcraft` files (docs/formats/modelcraft.md): text model definition, binary mesh, geometry references with content hashes, the journal; atomic saves; autosave storage for crash recovery. Bytes in, bytes out; the file system only in `fs` and `recovery`. Result files later. | started |
| L6 | `modelcraft-engine` | Commands (`CommandSpec`), `Session::execute` with rollback and the panic guard, scripts. | started |
| L7 | `modelcraft-ui-egui`, `modelcraft-mcp` | Desktop UI (egui + wgpu viewport, model tree, commands, status bar) and control channel requests (docs/control-protocol.md); MCP server. | ui started |
| testkit | `modelcraft-testkit` | Test tooling, a dev-dependency only: the meshing test corpus (`tests/corpus/*/recipe.json`, analytic shapes) and its runner (`cargo xtask corpus`). | started |
| apps | `modelcraft`, `modelcraft-cli`, `modelcraft-web` | Desktop, command line, browser. | desktop, cli started |

Rules beyond the layer order:
- Nothing below L7 uses egui, eframe, winit, wgpu or rfd.
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
   Diazzi et al. 2023 (implicit Steiner points on indirect predicates, gift-wrapping facet
   recovery; `modelcraft-mesh::cdt`, command `mesh.tet.generate`), so every boundary triangle and
   its B-rep face tag survives. Then constrained Delaunay refinement to a
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
  exist when it is made, and a definition still referred to (a load listed by a step included)
  cannot be deleted; what depends on
  the mesh is checked when the model is resolved.
- **Steps** list the boundary conditions and loads active in them. Nothing propagates from one
  step to the next.
- **Units** are a label (`mm-t-s-N-MPa`, …); numbers are never converted, and the check warns
  when a modulus, density or gravity is implausible for the chosen system.
- **Check and resolve.** `Model::check` lists errors and warnings, each naming its item (a set
  member not on the mesh, an element with no section or in two, a moment on solid nodes, a
  missing density for a frequency step, …). `Model::resolve` returns what writers and the solver
  read besides the definitions: every set resolved (each element then lies in exactly one
  section's set), the node label of every reference point, and every force, moment and traction
  as nodal values. It fails when the check finds an error.
- **Loads that keep their meaning on any mesh.** A force or moment on a set is a total by
  default: split by the consistent nodal weights `∫ N_i dA` of the set's faces (Gauss quadrature
  on each face's own linear or quadratic geometry), or equally over a set of nodes only. Applying
  it at every node is an explicit, mesh-dependent opt-in (`per_node`). A traction is force per
  area on faces. The expansion lives in `resolve`, so the deck writers and the solver share it.
- **Reference point labels** are stored with the point: chosen from the top of the label range
  when it is created, so remeshing or adding nodes and points never renumbers them.
- **Files.** A model deserializes by replaying its contents through the same checked methods the
  commands use, so a file cannot hold what commands would refuse.

## Implicit solver

`modelcraft-solver` solves a model's linear static and frequency steps (`solve.static`,
`solve.frequency`; `solve.probe` reads the stored results). Methods and papers are cited in the
module docs; ADR 0001 records why the linear algebra is our own.

1. **System.** `Model::resolve()`; each solid section's elements take their material's
   elasticity matrix and the section's formulation (`modelcraft_fem::element::ELEMENT_TYPES`, one
   row per element kind). Couplings, reference points, moments, shells and beams return "not
   supported yet". The step's boundary conditions prescribe translations (rotations do not exist
   on solid nodes); its loads become a load vector: forces and tractions from
   `Resolved::nodal_loads`, pressure as consistent face loads, gravity and body forces through the
   element shape functions.
2. **Assembly** on the node graph (3 × 3 blocks, elements in label order, so sums are
   reproducible bit for bit).
3. **Ordering and elimination.** Nested dissection of the nodes by coordinate bisection; free
   degrees of freedom numbered in that order; prescribed values moved to the right-hand side (no
   penalty).
4. **Static solution.** Up-looking sparse Cholesky (the factor size is computed and capped before
   any value), reactions K u − f, integration-point stresses extrapolated to the nodes and
   averaged, von Mises. A zero pivot names the node and direction that can move freely.
5. **Frequency.** Shift-invert block Lanczos (block 6, full reorthogonalization) on
   K − σM, σ = 0 for a supported structure and slightly negative for a free one, so rigid-body
   modes come out at zero; mass-normalized shapes, participation factors and effective masses.
6. **Results** stay in the session per step, stamped with the session revision they were solved
   at; `Session::result` hands out only those of the current revision, so an edit makes them
   stale and undoing back to the solved state makes them current again.

Verification (`crates/solver/tests/verification.rs`, analytic only): displacement and traction
patch tests for every element and formulation (exact to round-off), cantilever tip deflection
against Timoshenko with convergence rates of the linear elements, Lamé's cylinder, Kirsch's
plate with a hole (Kt → 3), cantilever and free–free beam frequencies with six rigid-body modes.

## Commands

Every user-visible action is a `CommandSpec` in `crates/engine/src/cmd/<area>.rs` with a JSON
form. `Session::execute` clones the model, runs the command under `catch_unwind`, and restores
the model on an error or a panic; commands that changed the model are journalled, so the
journal replays as a script. Each journalled command is also an undo step (the state before it):
`edit.undo` restores that state and moves the command off the journal onto the redo stack,
`edit.redo` puts it back, so the journal always rebuilds the current model. History is bounded by
step count and by an estimate of the memory the kept states hold (oldest steps go first).
Commands of kind `Session` (`file.*`, `edit.*`) are never journalled. `file.save` and `file.open`
write and read `.modelcraft` files; the desktop app autosaves unsaved work for `file.recover`.

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
