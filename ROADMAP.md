# Roadmap

ModelCraft's goal is CAE modeling on par with Abaqus/CAE, Nastran pre-processors and LS-PrePost,
with hexahedral meshing as the centre of gravity, plus its own linear solver. Abaqus/CAE is the
reference for scope (its documentation is excellent and its solver strong); its application is
where we aim to be clearly better: parametric geometry from SolidCraft, sets that survive
remeshing, automatic hex partitioning, undo, a text-diffable model and agent control.
The capability checklist is [docs/parity.md](docs/parity.md).

Order (revised 2026-10-13): one end-to-end slice first — *SolidCraft part → named faces → sets →
mesh → material and section → BCs and loads → static and frequency steps → Abaqus, Nastran and
LS-DYNA decks and our own solve → contour plot* — on tet meshes, so every hex mesher lands in a
pipeline that already produces solvable, verified models. Then hex: structured (sweep and mapped,
with partitions proposed automatically) before octree all-hex, because that is how analysts get
hex meshes today and it gives the best elements where stresses are read.

| Milestone | Scope | Status |
|---|---|---|
| M0 Foundation | Workspace, quality gates, FE model, command engine with rollback and panic guard, CLI, geometry boundary on SolidCraft | done |
| M1 Geometry in and surface preparation | Open `.solidcraft` designs with persistent face names, assemblies placed (done); face-tagged 3MF (done); test corpus, `cargo xtask corpus` (done); exact predicates (done); quality metrics and certified validity gate (done); weld, closed/manifold/orientation checks, feature edges | in progress |
| M2 Tet meshing and size field | Size field (global, per face and edge seeds, curvature and proximity); Bowyer–Watson Delaunay (exact, symbolic perturbation, verified; `modelcraft-mesh::delaunay`, done); robust CDT boundary recovery (Diazzi et al. 2023); Delaunay refinement, ODT smoothing, sliver removal; tet4 and tet10 with mid-side nodes on the B-rep; mesh verify per set | in progress |
| M3 First deck | Sets and surfaces bound to SolidCraft face, edge, vertex and body names, resolved after meshing; materials (isotropic and orthotropic elastic, density) with a unit-system label and consistency check; sections (solid, shell, beam profiles) assigned by set; element formulation per region mapped to each solver; BCs (fixed, displacement, symmetry); loads (force and moment on reference points, pressure, gravity); reference points with kinematic and distributing coupling; steps (linear static, frequency) with explicit per-step load and BC tables and output presets; writers for Abaqus input, Nastran SOL 101/103 and LS-DYNA implicit; the `.modelcraft` file | planned |
| M4 Own solver and first results | Linear static and modal solver (tet4/10, hex8, hex20; sparse direct, Lanczos), verified against analytic solutions; deformed shape, contours, mode shapes, probes, reaction sums | planned |
| M5 Structured hex | 2D quad meshing of faces; mapped hex of six-sided regions; sweep meshing; conforming interfaces and tet–hex transition; automatic decomposition into sweepable and mappable regions, proposed as SolidCraft splits | planned |
| M6 Automatic hex | Octree/grid-dual all-hex for any part; hex-dominant by tet recombination; polycube for blocky parts; smoothing, untangling, boundary padding, layers through thin walls; mesh editing and orphan-mesh import | planned |
| M7 Assemblies and contact | Part instances from SolidCraft occurrences; tie between non-matching meshes; general and pairwise contact with automatic pair detection, friction; bolt pretension, springs, point masses; amplitudes, multiple steps; nonlinear static steps in decks | planned |
| M8 Materials, explicit, shells, composites | Plasticity, hyperelasticity, damage data per solver; explicit dynamics decks (Abaqus/Explicit, LS-DYNA); midsurface shells; composite layups and material orientations, continuum shells on swept hex; heat transfer | planned |
| M9 Post-processing and deck readers | Results readers for public formats (Nastran OP2, LS-DYNA d3plot, Abaqus `.fil`); XY and path plots, cuts, display groups, free-body forces; deck readers that keep unknown keywords; mesh convergence studies | planned |
| Continuous: desktop and agents | Desktop app (viewport, model tree, commands, control channel: done), MCP server, browser build, command schema as the public API | started |
