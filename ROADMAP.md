# Roadmap

ModelCraft's goal is CAE pre-processing on par with Abaqus/CAE, Nastran pre-processors and
LS-PrePost, with hexahedral meshing as the centre of gravity, plus its own linear solver.

| Milestone | Scope | Status |
|---|---|---|
| M0 Foundation | Workspace, quality gates (`cargo xtask ci`), FE model, command engine with rollback and panic guard, CLI, geometry boundary on SolidCraft's kernel and tessellator | done |
| M1 Surface preparation | Geometry in: open `.solidcraft` designs, rebuilt by SolidCraft, with persistent face names (done); face-tagged 3MF files (`geometry.import.mesh`, docs/formats/tagged-mesh.md, done); the meshing test corpus: 16 SolidCraft recipes and 5 analytic shapes, `cargo xtask corpus` → docs/meshing/corpus.md (done); exact predicates (Shewchuk adaptive, symbolic perturbation, indirect LPI/TPI, done); weld, closed/manifold/orientation checks and feature edges on the tessellated boundary; mesh quality metrics and the certified validity gate (`modelcraft-mesh::quality`, `mesh.quality`, done) | in progress |
| M2 Tetrahedral meshing | Bowyer–Watson Delaunay, robust constrained Delaunay boundary recovery (Diazzi et al. 2023), Delaunay refinement to a size field, ODT smoothing, Klingner–Shewchuk sliver removal; linear and quadratic tets | planned |
| M3 Hexahedral meshing (core) | Octree/grid dual all-hex (valid on any input), then hex-dominant by tet recombination, then polycube for blocky parts; see docs/meshing/literature.md | planned |
| M4 Model definition | Materials, sections, sets and surfaces, boundary conditions, loads, steps, output requests | planned |
| M5 Solver decks | Abaqus input, Nastran bulk data, LS-DYNA keyword: writers first, then readers | planned |
| M6 Linear solver | Element library, sparse assembly, linear static and modal analysis, verified against analytic solutions | planned |
| M7 Desktop and agents | egui desktop app, control channel, MCP server, browser build | started: desktop app (viewport, model tree, commands, status bar) and control channel |
| M8 Post-processing | Results display: deformed shape, contours, probes | planned |
