# Element library

ModelCraft's finite element library (`crates/fem`), family by family, with status. The scope is
modelled on what a full CAE code offers (Abaqus's element library is the reference for breadth);
names, formulations and tests are our own, written from the published literature. Deck writers map
each of our elements to each solver's name (see `crates/decks`). Simple elements first: 1D and 2D,
then shells, special-purpose and multiphysics elements.

Status: **done** (on main, verified against analytic solutions), **in progress** (an agent task is
open), **planned**.

Every element builds on the shared foundation of docs/fem.md: it declares its nodal degrees of
freedom (`DofKind`: translations, rotations, temperature, pressure) and condensed internal ones
(`ElementDofs`), the solvers number them with `DofLayout`, and its interpolation comes from the
shape-function library (`Basis`: line2/3, tri3/6, quad4/8/9, tet4/10, wedge6/15, pyramid5,
hex8/20/27 with first and second derivatives).

## Point and discrete elements

| Element | Use | Status |
|---|---|---|
| Point mass | Lumped mass at a node or reference point (isotropic or per direction) | done (implicit, explicit; Abaqus MASS, Nastran CONM1, LS-DYNA ELEMENT_MASS; per direction: Nastran only) |
| Rotary inertia | Inertia tensor at a 6-DOF node | done (implicit; Abaqus ROTARYI, Nastran CONM1, LS-DYNA ELEMENT_INERTIA; explicit when it has rotations) |
| Spring to ground / two-point spring | Linear spring in a fixed direction or along the line of nodes | done (implicit, explicit translational; Abaqus SPRING1/2/A, Nastran CELAS2, LS-DYNA ELEMENT_DISCRETE; skew fixed directions: LS-DYNA only) |
| Dashpot to ground / two-point | Linear viscous damper (dynamic steps; ignored in static) | done (explicit; ignored with a warning in static and frequency steps; Abaqus DASHPOT1/2/A, Nastran CDAMP2, LS-DYNA ELEMENT_DISCRETE) |
| Nonstructural mass | Mass spread over a region without stiffness | planned |
| Connectors | Kinematic joints (hinge, slot, universal…) with elasticity, damping, stops | planned (M7) |

## 1D elements

| Element | Use | Status |
|---|---|---|
| Truss, 2-node (2D and 3D) | Axial members, cables (compression allowed) | done (rod2: implicit, explicit; T3D2, CROD, LS-DYNA truss beam; a plane truss is a space truss with its out-of-plane motion held) |
| Truss, 3-node (2D and 3D) | Curved axial members | done (rod3: implicit, explicit; Abaqus T3D3; no Nastran or LS-DYNA counterpart) |
| Beam, 2-node Timoshenko, 3D | Frames, stiffeners; shear deformable | in progress (shells and beams) |
| Beam, 2-node Euler–Bernoulli (cubic), 2D and 3D | Slender frames; exact for end-loaded prismatic members | planned (1D elements II) |
| Beam, 3-node Timoshenko (quadratic), 2D and 3D | Curved and stocky beams | planned (1D elements II) |
| Beam, 2D (planar frames) | In-plane frames with 3 DOFs per node | planned (1D elements II) |
| Pipe / elbow | Pipes with internal pressure, ovalisation | planned (1D elements II) |
| Open-section beams with warping | Thin-walled sections in torsion | planned |
| Rebar / stringers | Reinforcement along edges or embedded | planned |

## 2D elements

| Element | Use | Status |
|---|---|---|
| Plane stress tri3, tri6, quad4, quad8 | Thin plates loaded in their plane | in progress (2D elements) |
| Plane strain tri3, tri6, quad4, quad8 | Long sections (dams, tunnels, extrusions) | in progress (2D elements) |
| Axisymmetric tri3, tri6, quad4, quad8 | Revolved solids (shafts, vessels, discs) | in progress (2D elements) |
| Incompatible-mode and reduced-integration quads | Bending-accurate and cheap variants | in progress (2D elements) |
| Generalized plane strain | Plane strain with a free out-of-plane strain | planned |
| Axisymmetric with twist | Revolved parts under torsion | planned |
| Axisymmetric shells (2-node, 3-node) | Thin revolved shells (tanks, domes) | planned |
| Hybrid (mixed u–p) variants | Nearly incompressible materials | planned |

## Shells and membranes

| Element | Use | Status |
|---|---|---|
| Shell quad4 (MITC4), triangle (locking-free) | Thin and thick shells | in progress (shells and beams) |
| Shell quad8 / tri6 | Curved thick shells | planned |
| Continuum shells (8-node, on swept hex) | Thick-to-thin shells, composites through thickness | planned (M8) |
| Membranes (tri3, quad4, quad8) | Fabrics, inflatables, skins without bending | planned |
| Surface elements | Massless surfaces for loads and contact | planned |
| Composite layups on shells | Ply tables, orientations | planned (M8) |

## 3D solids

| Element | Use | Status |
|---|---|---|
| tet4, tet10 | Free meshes of any part | done |
| hex8 (full, incompatible modes), hex20 (full, reduced) | Structured and swept meshes | done |
| wedge6 | Swept meshes from triangles, transitions | done |
| hex8 one-point with hourglass control, nodal-pressure tet4 (explicit) | Explicit dynamics | done |
| wedge15, pyramid5/13 | Quadratic transitions, tet–hex transition | planned (M5); wedge15 and pyramid5 shape functions done |
| Hybrid (mixed u–p) solids | Nearly incompressible materials (rubber) | in progress (FEM foundation, phase C); pressure DOF kind done |
| Modified quadratic tet (tet10 for contact/plasticity) | Robust quadratic tets | planned |

## Special-purpose and multiphysics

| Element | Use | Status |
|---|---|---|
| Cohesive (2D, 3D) | Adhesive layers, delamination | planned |
| Gasket | Through-thickness gasket behaviour | planned |
| Infinite elements | Unbounded domains | planned |
| Heat transfer (all shapes), coupled temperature–displacement | Thermal and thermal-stress analyses | in progress (FEM foundation, phase B); temperature DOF kind done |
| Acoustic | Fluid acoustics, structural–acoustic coupling | planned |
| Rigid elements | Rigid bodies from meshes | planned (M7) |
