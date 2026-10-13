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
| L0 | `modelcraft-model` | Solver-neutral FE model: nodes, elements by label; materials, sections, sets, BCs, loads and steps as they land. Knows no file format. | started |
| L1 | `modelcraft-geometry` | The only door to SolidCraft: opens `.solidcraft` designs (rebuilt from their timeline, bodies with persistent face names), B-rep bodies, measures, tessellation into a face-tagged `SurfaceMesh`; face-tagged 3MF files read into the same type without the kernel (docs/formats/tagged-mesh.md). No SolidCraft type in its API. | started |
| L1 | `modelcraft-mesh` | Surface preparation (weld, watertightness, feature edges), robust predicates, Delaunay tetrahedralization and refinement, Voronoi-based optimization, hex meshing, quality metrics. | started |
| L1 | `modelcraft-solver` | Element formulations, assembly, sparse linear static and modal solution. | planned |
| L1 | `modelcraft-decks` | Abaqus input, Nastran bulk data and LS-DYNA keyword readers and writers, to and from `model`. | planned |
| L1 | `modelcraft-render` | Display data (meshes, contour maps) without a GPU dependency. | planned |
| L2 | `modelcraft-doc` | The document: geometry, mesh, model, results, history. | planned |
| L3 | `modelcraft-io` | `.modelcraft` files, result files. | planned |
| L4 | `modelcraft-engine` | Commands (`CommandSpec`), `Session::execute` with rollback and the panic guard, scripts. | started |
| L5 | `modelcraft-ui-egui`, `modelcraft-mcp` | Desktop UI, control channel and MCP server. | planned |
| apps | `modelcraft`, `modelcraft-cli`, `modelcraft-web` | Desktop, command line, browser. | cli started |

Rules beyond the layer order:
- Nothing below L5 uses egui, eframe, winit, wgpu or rfd.
- Only `modelcraft-geometry` uses `solidcraft-*`, pinned by rev in the workspace `Cargo.toml`.
  SolidCraft's `[patch.crates-io]` (its fixed truck crates) does not reach dependents, so the
  workspace repeats it from the same rev; bump both together.
- Everything below the apps builds for `wasm32-unknown-unknown`.

## Meshing pipeline

1. **Boundary surface.** SolidCraft tessellates each body; every triangle carries its B-rep face,
   so feature edges and face-based loads and BCs survive meshing. Faces of an opened design also
   carry SolidCraft's persistent names (`F2:side:l4`, from the feature and sketch curve that made
   them), so a load on a named face follows it when the design is edited and opened again.
2. **Surface preparation.** Weld coincident vertices, check the surface is closed, manifold and
   consistently oriented, find feature edges (face boundaries, sharp dihedral angles).
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
5. **Quality.** Scaled Jacobian, aspect ratio, dihedral angles; every generated mesh is checked
   for positive Jacobians and conserved volume before it reaches the model.

## Commands

Every user-visible action is a `CommandSpec` in `crates/engine/src/cmd/<area>.rs` with a JSON
form. `Session::execute` clones the model, runs the command under `catch_unwind`, and restores
the model on an error or a panic; commands that changed the model are journalled, so the
journal replays as a script.
