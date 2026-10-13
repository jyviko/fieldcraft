# Features

Generated from the capability catalogue (`crates/engine/src/catalogue.rs`) by
`cargo run -p modelcraft-cli -- features > docs/features.md`; a test fails when it is out of date.
Every entry is a toolbar button. **Ready** entries run their command; **coming soon** entries are
shown greyed with the roadmap milestone that brings them (ROADMAP.md).

23 ready, 75 coming soon.

## Geometry

| Panel | Capability | Status | Command |
|---|---|---|---|
| Import | Open Design — Open a SolidCraft design: its bodies with persistent face names | ready | `geometry.open` |
| Import | Tagged Mesh — Read a face-tagged surface mesh (3MF) | ready | `geometry.import.mesh` |
| Inspect | Bodies — List the open design's bodies, volumes and named faces | ready | `geometry.bodies` |
| Inspect | Tessellate — Tessellate a body into a face-tagged boundary surface | ready | `geometry.tessellate` |
| Inspect | Check Surface — Weld, closed, manifold, orientation and self-intersection checks; feature edges | ready | `mesh.surface.check` |
| Prepare | Partition — Split bodies and faces as SolidCraft features | coming soon (M5) |  |
| Prepare | Datum Plane — Construction planes for partitions | coming soon (M5) |  |
| Prepare | Midsurface — Extract midsurfaces of thin parts for shell meshes | coming soon (M8) |  |

## Mesh

| Panel | Capability | Status | Command |
|---|---|---|---|
| Size | Global Size — One element size for the whole part | coming soon (M2) |  |
| Size | Seed Edges — Element counts, sizes and bias per edge and face | coming soon (M2) |  |
| Size | Curvature — Refine by curvature and proximity | coming soon (M2) |  |
| Tet | Tet Mesh — Delaunay tetrahedral mesh of a body, boundary recovered and refined | coming soon (M2) |  |
| Tet | Quadratic — Second-order elements with mid-side nodes on the geometry | coming soon (M2) |  |
| Tet | Delaunay Points — Delaunay tetrahedralization of a point list (debugging aid) | ready | `mesh.delaunay.points` |
| Structured Hex | Quad Faces — All-quad meshes of faces on shared edge nodes | coming soon (M5) |  |
| Structured Hex | Mapped Hex — Mapped hex mesh of six-sided regions | coming soon (M5) |  |
| Structured Hex | Sweep Hex — Hex mesh swept along extrusions and revolutions | coming soon (M5) |  |
| Structured Hex | Decompose — Find sweepable and mappable regions and propose the splits | coming soon (M5) |  |
| Structured Hex | Transition — Conforming tet–hex interfaces with pyramids | coming soon (M5) |  |
| Automatic Hex | All-Hex — Octree dual all-hex mesh of any part | coming soon (M6) |  |
| Automatic Hex | Hex-Dominant — Hex-dominant mesh by recombining tets | coming soon (M6) |  |
| Automatic Hex | Polycube — Polycube hex mesh of blocky parts | coming soon (M6) |  |
| Improve | Smooth — Smooth node positions, keeping every element valid | coming soon (M6) |  |
| Improve | Untangle — Repair inverted elements | coming soon (M6) |  |
| Improve | Layers — Boundary padding and layers through thin walls | coming soon (M6) |  |
| Edit | Node — Add a node | ready | `mesh.node.add`, `mesh.node.remove` |
| Edit | Element — Add an element on existing nodes | ready | `mesh.element.add`, `mesh.element.remove` |
| Edit | Merge Nodes — Merge coincident nodes | coming soon (M6) |  |
| Edit | Remesh Region — Remesh a region in place; split and collapse elements | coming soon (M6) |  |
| Edit | Orphan Mesh — Import a mesh without geometry | coming soon (M6) |  |
| Verify | Quality — Quality metrics and the certified validity check, failing elements listed | ready | `mesh.quality` |
| Verify | Verify Set — Quality and validity per set | coming soon (M2) |  |
| Verify | Tag Geometry — Tag elements, faces, edges and nodes with the geometry they mesh | ready | `mesh.tag`, `mesh.tag.clear` |

## Properties

| Panel | Capability | Status | Command |
|---|---|---|---|
| Materials | Material — Elastic (isotropic, orthotropic) material with density | ready | `model.material.create`, `model.material.edit`, `model.material.delete` |
| Materials | Plasticity — Plastic hardening data | coming soon (M8) |  |
| Materials | Hyperelastic — Hyperelastic material models | coming soon (M8) |  |
| Materials | Damage — Damage initiation and evolution | coming soon (M8) |  |
| Materials | Thermal — Conductivity, specific heat and expansion | coming soon (M8) |  |
| Materials | Equation of State — Polynomial, ideal gas, Mie–Grüneisen, JWL and tabulated equations of state | coming soon (M11) |  |
| Sections | Section — Solid, shell, beam or rod section assigned to a set, with its element formulation | ready | `model.section.create`, `model.section.edit`, `model.section.delete` |
| Sections | Composite — Composite layups and continuum shells | coming soon (M8) |  |
| Sections | Orientation — Material orientations | coming soon (M8) |  |
| Sets | Set — A set of nodes, elements and faces, or of named geometry that survives remeshing | ready | `model.set.create`, `model.set.edit`, `model.set.delete`, `model.set.resolve` |
| Model | Units — The model's unit system (never converted) | ready | `model.units` |

## Assembly

| Panel | Capability | Status | Command |
|---|---|---|---|
| Instances | Instances — Part instances placed from SolidCraft occurrences | coming soon (M7) |  |

## Steps

| Panel | Capability | Status | Command |
|---|---|---|---|
| Steps | Step — A linear static or frequency step with its output | ready | `model.step.create`, `model.step.edit`, `model.step.delete` |
| Steps | Activate — Choose which loads and boundary conditions act in a step | ready | `model.step.activate` |
| Steps | Step Table — The step by load and boundary condition table | ready | `model.activity` |
| Procedures | Amplitude — Time and frequency amplitudes | coming soon (M7) |  |
| Procedures | Nonlinear — Nonlinear static steps | coming soon (M7) |  |
| Procedures | Explicit — Explicit dynamic steps | coming soon (M8) |  |
| Procedures | Heat Transfer — Heat transfer steps | coming soon (M8) |  |

## Loads & BCs

| Panel | Capability | Status | Command |
|---|---|---|---|
| Boundary Conditions | Boundary Condition — Fixed, displacement or symmetry condition on a set or reference point | ready | `model.boundary_condition.create`, `model.boundary_condition.edit`, `model.boundary_condition.delete` |
| Loads | Load — Force, moment, pressure, traction, gravity or body force | ready | `model.load.create`, `model.load.edit`, `model.load.delete` |
| Flow | Fluid Domain — The fluid grid voxelised from the face-tagged surface | coming soon (M12) |  |
| Flow | Inlet & Outlet — Velocity inlets, pressure outlets and walls | coming soon (M12) |  |

## Interactions

| Panel | Capability | Status | Command |
|---|---|---|---|
| Constraints | Reference Point — A reference point for couplings, loads and boundary conditions | ready | `model.reference_point.create`, `model.reference_point.edit`, `model.reference_point.delete` |
| Constraints | Coupling — Kinematic or distributing coupling of a set to a reference point | ready | `model.coupling.create`, `model.coupling.edit`, `model.coupling.delete` |
| Constraints | Tie — Tie non-matching meshes together | coming soon (M7) |  |
| Contact | General Contact — Contact between everything that touches, with friction | coming soon (M7) |  |
| Contact | Contact Pair — Contact between two surfaces | coming soon (M7) |  |
| Contact | Find Contacts — Detect contact pairs automatically | coming soon (M7) |  |
| Contact | Rigid Wall — Rigid walls for explicit dynamics | coming soon (M10) |  |
| Connectors | Bolt Load — Bolt pretension | coming soon (M7) |  |
| Connectors | Spring — Springs and dashpots | coming soon (M7) |  |
| Connectors | Point Mass — Point masses and inertia | coming soon (M7) |  |
| Connectors | Connector — Connector elements between points | coming soon (M7) |  |
| Coupled | FSI Interface — The fluid–structure interface | coming soon (M13) |  |

## Solve

| Panel | Capability | Status | Command |
|---|---|---|---|
| Check | Check Model — Name every missing or bad definition before writing or solving | ready | `model.check` |
| Decks | Abaqus Input — Write an Abaqus input file (.inp) | coming soon (M3) |  |
| Decks | Nastran Deck — Write Nastran bulk data (SOL 101, 103) | coming soon (M3) |  |
| Decks | LS-DYNA Deck — Write an LS-DYNA keyword file | coming soon (M3) |  |
| Decks | Read Deck — Read a deck, keeping keywords ModelCraft does not know | coming soon (M9) |  |
| Implicit | Static — Linear static solve with ModelCraft's own solver | coming soon (M4) |  |
| Implicit | Frequency — Natural frequencies and mode shapes | coming soon (M4) |  |
| Explicit | Explicit — Explicit dynamics: central differences, contact, erosion | coming soon (M10) |  |
| Explicit | Blast & Impact — EOS-driven fluids with fluid–structure contact (ALE or SPH) | coming soon (M14) |  |
| Fluids | Lattice Boltzmann — Lattice Boltzmann flow on the CPU or the GPU | coming soon (M12) |  |
| Fluids | Fluid–Structure — Coupled fluid and structure solve | coming soon (M13) |  |

## Results

| Panel | Capability | Status | Command |
|---|---|---|---|
| Display | Deformed — The deformed shape | coming soon (M4) |  |
| Display | Contours — Contour plots of stress, strain and displacement | coming soon (M4) |  |
| Display | Mode Shapes — Mode shapes of a frequency step | coming soon (M4) |  |
| Display | Animate — Animate modes and time histories | coming soon (M9) |  |
| Query | Probe — Values at nodes and elements | coming soon (M4) |  |
| Query | Reactions — Reaction force and moment sums | coming soon (M4) |  |
| Query | Free Body — Free-body forces across a cut | coming soon (M9) |  |
| Plots | XY Plot — Plot a quantity against time or another | coming soon (M9) |  |
| Plots | Path Plot — Plot a quantity along a path | coming soon (M9) |  |
| Plots | Cut — Section cuts through the model | coming soon (M9) |  |
| Plots | Display Group — Show and hide parts of the model | coming soon (M9) |  |
| Plots | Streamlines — Streamlines and flow fields | coming soon (M12) |  |
| Files | Read Results — Read results in public formats | coming soon (M9) |  |
| Files | Convergence — Mesh convergence studies | coming soon (M9) |  |

## Tools

| Panel | Capability | Status | Command |
|---|---|---|---|
| Model | Model Info — Node and element counts and the model summary | ready | `model.info` |
| File | Open Model — Open a .modelcraft model | coming soon (M3) |  |
| File | Save Model — Save the model as a .modelcraft file | coming soon (M3) |  |
| History | Undo — Undo the last command | coming soon (M3) |  |
| History | Redo — Redo the command undone last | coming soon (M3) |  |
