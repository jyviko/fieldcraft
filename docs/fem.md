# FEM foundation: degrees of freedom and shape functions

What every element in `crates/fem` and every solver builds on. Code: `crates/fem/src/dof.rs`,
`crates/fem/src/basis.rs`, `crates/fem/src/jet.rs`.

## Degrees of freedom

Every unknown is a pair (node, `DofKind`):

| `DofKind` | Meaning | Model / deck number |
|---|---|---|
| `Ux`, `Uy`, `Uz` | translations along X, Y, Z | 1, 2, 3 |
| `Rx`, `Ry`, `Rz` | rotations about X, Y, Z | 4, 5, 6 |
| `P` | pressure (mixed u–p elements) | 8 |
| `T` | temperature | 11 |

The enum order (Ux, Uy, Uz, Rx, Ry, Rz, T, P) is the numbering order within a node.
`DofKind::from_number` / `number` convert to the 1-based numbers boundary conditions use;
`from_direction` / `direction` to the structural directions 0–5.

**Elements declare their unknowns** with `ElementDofs { node: DofSet, internal: Internal }`:
the kinds every node of the element carries (`DofSet::TRANSLATIONS` for solids, `PLANE` for
2D elements, `STRUCTURAL` for shells and beams, `TEMPERATURE` for heat transfer, translations
plus `PRESSURE` for continuous-pressure mixed elements) and the internal unknowns the element
condenses out itself (`incompatible_modes`, discontinuous `pressure`), which never reach the
global system. An element's matrices are ordered node by node, its `node` kinds in `DofKind`
order within a node (a shell's 24 × 24 stiffness: node 1 Ux Uy Uz Rx Ry Rz, node 2 …). Solid
elements: `Formulation::dofs()` / `Solid::dofs()`.

**Numbering** (`DofLayout`): collect what every node needs with `DofLayout::builder()`
(`element(labels, dofs)`, `node(label, kinds)` for constraints and boundary conditions that
need a kind no element gives, `point(label, kinds)` for reference points), then `build()`. A
node carries the union of what was asked for. Unknowns are numbered mesh nodes in label order,
each node's kinds in `DofKind` order, then the points in the order added; a slot's unknowns are
contiguous, so a node pair is one dense block in the assembled matrix. The same model always
gives the same numbering.

Lookups: `node(label)` / `slot(label)` (a slot is a position: mesh nodes `0..nodes()`, points
`point_slot(k)`), `dof(slot, kind)`, `range(slot)`, `kinds(slot)`, `locate(dof) -> (slot,
kind)`, `element_dofs(slots, kinds)`, `describe(dof)`, `stride()` (uniform layouts).

Trusses declare `fieldcraft_fem::truss::DOFS` (translations, on `Basis::Line2`/`Line3`).
Point masses, springs and dashpots are not mesh elements: the implicit solver asks the layout
for what each needs at its nodes (`fieldcraft_solver::discrete::needs`: translations for a mass
or a translational spring, rotations for an inertia or a torsional spring; a reference point
they use is a point with six) and finds its unknowns with `discrete::dof_of`.

Solvers: `fieldcraft_solver::system::Mesh::layout` numbers the implicit system (mesh nodes,
then six per coupled reference point); assembly (`sparse::add_block(k, layout, slots, kinds,
data)`, `add_scalar`, `add_diagonal`) works for any mix of kinds per node, so a solid sharing
nodes with a shell adds its translation block into the shell nodes' six-unknown blocks.
Boundary conditions and loads address (node, kind): a boundary condition holds the kinds a node
carries, and a moment needs rotations. The explicit solver has solid nodes only and checks that
its layout is the uniform node-major one (`NODE_DOFS = 3`).

## Shape functions

`Basis` is every family's interpolation:

| Family | Reference element | Nodes |
|---|---|---|
| `Line2`, `Line3` | ξ ∈ [−1, 1] | ends −1, 1; line3 adds the middle |
| `Tri3`, `Tri6` | ξ, η ≥ 0, ξ + η ≤ 1 | corners (0,0), (1,0), (0,1); mid-sides of 1-2, 2-3, 3-1 |
| `Quad4`, `Quad8`, `Quad9` | [−1, 1]² | corners counter-clockwise from (−1,−1); mid-sides of 1-2, 2-3, 3-4, 4-1; centre |
| `Tet4`, `Tet10` | unit tetrahedron | corners (0,0,0), (1,0,0), (0,1,0), (0,0,1); mid-sides of 1-2, 2-3, 3-1, 1-4, 2-4, 3-4 |
| `Wedge6`, `Wedge15` | triangle × ζ ∈ [−1, 1] | bottom 1-3, top 4-6; mid-sides of the bottom edges, top edges, vertical edges |
| `Pyramid5` | base [−1, 1]² at ζ = 0, apex (0, 0, 1) | base corners as quad4, then the apex |
| `Hex8`, `Hex20`, `Hex27` | [−1, 1]³ | corners 1-4 at ζ = −1, 5-8 above; hex20 mid-sides (bottom, top, vertical edges); hex27 face centres in `ElementKind::Hex8.faces()` order, then the centre |

Node orders are the model's (`ElementKind::edges()` / `faces()` are the single source):
corners, mid-side nodes in edge order, face nodes, body nodes. `Basis::of(ElementKind)` maps a
model element; `nodes()` gives natural node coordinates; `dim()`, `count()`, `corners()`,
`degree()` (the complete polynomial degree reproduced: 1 for linear families including the
rational pyramid, 2 for the quadratic ones).

Evaluation at a natural point `xi: [f64; 3]` (unused coordinates are zero):

- `eval(xi) -> Shape`: values `n` and first derivatives `dn[i] = ∂Nᵢ/∂(ξ, η, ζ)`. tet4, tet10,
  wedge6, hex8, hex20 use the hand-written functions of `shape.rs` (the solid kernels' path).
- `eval2(xi) -> Shape2`: adds `d2n[i][a][b] = ∂²Nᵢ/∂ξₐ∂ξ_b`, from second-order forward-mode
  automatic differentiation of the same formulas (`jet::Jet`): exact to round-off.
- `gradients(&shape, xyz) -> Gradients`: physical `dx[i] = ∇ₓNᵢ = J⁻¹ ∇_ξNᵢ` and `det J`, with
  J_ab = ∂x_b/∂ξₐ; an inverted or collapsed element is `Error::Degenerate`.
- `hessians(&shape2, xyz) -> Hessians`: physical `d2x[i]` =
  J⁻¹ (H_ξ − Σ_b (∂N/∂x_b) X_b″) J⁻ᵀ with X_b″ = Σᵢ x_{i,b} H_ξ(Nᵢ), the map's second
  derivative (T. J. R. Hughes, *The Finite Element Method*, 2000, §3.9, §4.2), and
  `laplacian(i)`. Without the X″ term a field linear in x would have a nonzero Hessian on a
  curved element.

A family of dimension d maps onto the first d physical coordinates (plane elements in x–y,
lines along x). Elements embedded in a higher dimension (shells, beams, trusses in 3D) pass
node coordinates in their own local frame.

## Verification (`basis::tests`, `dof::tests`, `jet::tests`)

For every family: Kronecker delta at the nodes; partition of unity (Σ N = 1, Σ ∇N = 0,
Σ H = 0); exact reproduction of every monomial up to `degree()` with its gradient and Hessian
in natural coordinates; first and second derivatives against central differences;
on an affine map, physical gradients, Hessians and Laplacians exact for a complete polynomial
of the family's degree; on a curved map, linear fields exact with zero Hessian, and the
physical Hessian of a quadratic interpolant consistent with finite differences of its physical
gradient. The solver and explicit migration to `DofLayout` was checked bit for bit: every
result of the solver, explicit, engine and io test suites hashes identically before and after.

## Heat transfer and thermal strain

A heat conduction element carries one temperature per node (`heat::HEAT_DOFS`, kind `T`);
`heat::Conduction` gives the conductivity ∫ ∇Nᵀ k ∇N dV (full rule), the capacity ∫ ρc N Nᵀ dV
and a source ∫ N Q dV (mass rule), and the heat flux −k∇T at the integration points;
`heat::face_weights` and `face_film` integrate surface flux and convection on tri3/tri6/quad4/quad8
faces. The solver (`fieldcraft_solver::heat`) numbers the temperatures with
`DofLayout::uniform(…, DofSet::TEMPERATURE)` and assembles with the same `sparse::add_block`.

Thermal strain ε_th = α (T − T_ref) along the material axes enters a solid as the load
`Solid::thermal_load(d, &ThermalStrain { alpha, delta })` = ∫ Bᵀ D ε_th dV and its stress as
`Solid::stresses_with(d, u, Some(&thermal))` = D (ε − ε_th); `delta` is the nodal T − T_ref.
2D elements have the same pair (`Plane::thermal_load`, `Plane::stresses_with`): plane stress
leaves εzz free (σzz = 0), plane strain holds it (σzz = −D₃·ε_th), axisymmetric elements strain
the hoop direction too. Trusses (rod2, rod3) take the axial part Σ αₖ tₖ² ΔT along their tangent (`Truss::thermal_load`,
`Truss::stresses_with`). Shells and beams will provide the same two functions for their strains;
springs, dashpots and point masses have no thermal strain.

Explicit dynamics takes a step's temperature loads as a thermal strain rate per element,
α ΔT / end time (the change reached linearly over the step, like a prescribed displacement, so
there is no thermal shock), subtracted from each increment's strain increment before the
constitutive update, so it feeds J2 and Johnson–Cook plasticity alike; isotropic expansion only
(the rate is the same in any frame). Explicit trusses take no thermal strain yet (a warning
says so).

Model side: `Material::thermal` (`conductivity`, `specific_heat`, `expansion`, each isotropic
or per axis, and `reference_temperature`), `BoundaryCondition::Temperature`, loads
`SurfaceFlux`, `Convection`, `HeatSource` (heat transfer) and `Temperature` (a temperature field
in a static step), procedures `HeatTransfer { transient }` and `CoupledTemperatureDisplacement`.
Commands: `solve.heat`, and `solve.static` for coupled steps. The deck writers refuse heat
transfer by name for now.
## Hybrid (mixed u–p) solids

A section with `"formulation": {"hybrid": true}` gives `Formulation::Hybrid { pressures }`
(`crate::hybrid`): the isotropic elasticity splits as D = D_dev + K m mᵀ, the mean stress is an
independent field discontinuous between elements (hex20: 1, x, y, z; hex8 and tet10: constant),
and it is condensed out element by element, p̂ = K M⁻¹ Gᵀ u, leaving the stiffness
K_dev + K G M⁻¹ Gᵀ. `ElementDofs.internal.pressure` counts the condensed unknowns; nothing
global changes, and Cholesky still applies. Stresses are D_dev ε + p m with the element's
pressure field. The choice of pressure space follows the inf-sup analysis (docs in
`crates/fem/src/hybrid.rs`): Q2–P1disc and P2–P0 are stable; Q1–P0 does not lock but may show
checkerboard pressures; linear tetrahedra and wedges lock even so and are refused. A thermal
strain splits the same way: ∫ Bᵀ D_dev ε_th on the nodes and, through the pressure,
K G M⁻¹ ∫ N_p tr ε_th, so a heated nearly incompressible solid takes its bulk stress −3KαΔT
when confined. A continuous
nodal pressure (`DofKind::P`, Taylor–Hood) needs a symmetric indefinite factorization and is
planned.
