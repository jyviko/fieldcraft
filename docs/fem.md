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

## Nonlinear static analysis (finite strain)

A step `{"type": "nonlinear_static", …}` (`Procedure::NonlinearStatic`, solved by
`solve.nonlinear`, `fieldcraft_solver::nonlinear`) follows equilibrium in load increments with
Newton–Raphson iterations. The step's load factor λ runs from 0 to 1: dead loads are λ times
their values, a pressure follows the deformed faces with λ times its magnitude, and prescribed
displacements are λ times theirs (a linear ramp).

**Material point** (`fieldcraft_fem::constitutive`). Given the deformation gradient F and the
history committed at the end of the last converged increment, a material returns the
**Kirchhoff stress** τ = J σ and the **spatial tangent** c of its Lie derivative (L_v τ = c : d),
in Voigt order with engineering-shear columns, so c = D at F = I. This is the push-forward of
the second Piola–Kirchhoff stress and its tangent: τ = F S Fᵀ, c_ijkl = F_iI F_jJ F_kK F_lL
C_IJKL. History variables (`MaterialModel::history_len` per integration point) are read from the
committed slice and written to a separate one, which the solver commits when the increment
converges, so a cut-back increment leaves no trace. The materials:

| Material in a nonlinear step | Model |
|---|---|
| Elastic (isotropic or orthotropic), no option | St Venant–Kirchhoff: S = D E, E = ½(FᵀF − I). Right for large rotations with small strains; it softens without bound in large compression. |
| Isotropic elastic with `"hyperelastic": {"type": "neo_hookean"}` | Compressible neo-Hookean with the isochoric–volumetric split, W = μ/2 (Ī₁ − 3) + κ/2 (J − 1)², μ and κ from E and ν (C₁₀ = μ/2, D₁ = 2/κ). Its small-strain limit is the linear material, which linear steps use. |

"Isotropic elastic" keeps meaning the same thing everywhere: the neo-Hookean model is chosen
only by the explicit option, and explicit dynamics refuses a hyperelastic material by name.

**Elements** (`fieldcraft_fem::finite`), total Lagrangian over the reference element with its
own quadrature: f_a = ∫ Bₓ,aᵀ τ dV₀ and K_ab = ∫ (Bₓ,aᵀ c Bₓ,b + ∇ₓN_a·τ∇ₓN_b I) dV₀ with the
current gradients ∇ₓN = F⁻ᵀ∇₀N (material plus geometric stiffness), identical term by term to
∫ B₀ᵀ S dV₀ and its linearisation. Every element's tangent is checked against central
differences of its forces.

| Element | Nonlinear steps |
|---|---|
| tet4, tet10, wedge6, hex8, hex20 (full), hex20 (reduced) | total Lagrangian |
| hybrid hex8, hex20, tet10 | Simo–Taylor–Pister three-field form on the linear hybrid elements' pressure fields (θ the projection of J, p the projection of U′(θ)); needs the neo-Hookean option (St Venant–Kirchhoff has no volumetric split) |
| hex8 and quad4 incompatible modes | refused: their finite-strain form (enhanced assumed strain, Simo–Armero 1992) needs the modes as unknowns carried between iterations and hourglasses in large compression; use full integration, hybrid or quadratic elements |
| plane strain, axisymmetric (tri3, tri6, quad4, quad8) | total Lagrangian with F₃₃ = 1 or r/R (hoop column N/r, geometric term τ_θθ N_a N_b/r²) |
| plane stress | refused (σ_zz = 0 needs a local iteration for the thickness) |
| trusses, shells, beams | refused (large rotations of structural elements come later in M15) |

Without nonlinear geometry (`"nonlinear_geometry": false`) the elements are the linear ones:
every material is elastic today, so such a step is the linear solution reached by Newton (the
path the next materials, plasticity first, take with small strains).

**Follower pressure** (`fieldcraft_fem::face::follower_pressure`, `follower_edge_pressure`):
fᵢ = −p ∫ Nᵢ (x,ξ × x,η) dξ dη on the current face, with its load stiffness
K = p ∫ N_a (N_b,η [x,ξ]× − N_b,ξ [x,η]×). The load stiffness is not symmetric. The solver keeps
its symmetric part and leaves the skew part out, so the tangent stays symmetric for the
supernodal Cholesky factorization: the residual is exact (the converged answer does not
change), and over a closed surface or one whose boundary is held the skew parts of neighbouring
faces cancel (Schweizerhof & Ramm 1984), so nothing is lost there; elsewhere Newton may converge
linearly instead of quadratically, and the result says how large the left-out part is. An LU
factorization would double the factor's memory and lose the symmetric solver for a gain only in
the rate of convergence of open loaded surfaces; it is revisited with the symmetric indefinite
(LDLᵀ) factorization that arc-length and Taylor–Hood need. Without nonlinear geometry a pressure
is a dead load.

**Newton–Raphson** with the consistent tangent refactored every iteration, an optional line
search (off by default: on large elastic rotations the full step converges in fewer
iterations), and convergence when the force residual (∞-norm over the largest force seen), the
displacement correction (over the increment's largest displacement) and the energy (over the
first iteration's) are all below their tolerances, or the residual is round-off. Each increment
starts from the linear extrapolation of the last one. Automatic incrementation grows an
increment by half after two fast ones (≤ 4 iterations), shrinks it after a slow one (> 8), and
cuts it to a quarter when an element inverts, the residual is not finite or grows twice, the
tangent is not positive definite, or the iterations run out; the step stops with the load factor
and the reason at the minimum increment, the increment cap, or `Limits::max_newton_iterations`.

**Results**: the final displacements and reactions, the **Cauchy stress** and the
**logarithmic strain** (ln V, engineering shears) at the integration points and averaged at the
nodes, every increment's load factor, iterations and residuals, and a displacement frame per
converged increment (`results.frame`, the slider in the results panel).

**Couplings** stay the linear constraints of the linear solver (small rotations of the
reference points), with a warning; springs act along their fixed directions. Temperature and
centrifugal loads are refused by name in nonlinear steps.

**Decks**: Abaqus `*STEP, NLGEOM=YES|NO, INC=` with `*STATIC` (initial, period 1, minimum,
maximum) and `*CONTROLS, PARAMETERS=LINE SEARCH`; the neo-Hookean option as `*HYPERELASTIC, NEO
HOOKE`. LS-DYNA: the ramped implicit run with full Newton (`ilimit` 1, `maxref`) and
`*CONTROL_IMPLICIT_AUTO`; hyperelastic materials in nonlinear steps and steps without nonlinear
geometry are refused. Nastran refuses nonlinear steps (SOL 400) by name. Iteration caps and
tolerances are left to each solver's own controls, whose norms differ from ours.

**Verification** (`crates/solver/tests/nonlinear.rs`, `crates/fem/src/{constitutive,finite,face}.rs`,
and `tests/oracle/nl-*`): consistent tangents of both materials and of every element (central
differences); uniaxial stretch to λ = 2 of a neo-Hookean block in every element kind, plane
strain and axisymmetric (closed form); St Venant–Kirchhoff uniaxial stretch; the nonlinear patch
test on a distorted mesh (all solids, hybrid ones included); a 90° rigid rotation (zero stress);
the elastica (Bisshopp–Drucker) at P L²/(E I) = 1, 2, 5, 10 with hex20 (0.2–0.5 %); the inflation of a thick neo-Hookean tube by a follower pressure (Rivlin's closed form);
Newton's quadratic convergence; the linear limit; a finite-strain Cook's membrane is pending
its transcribed reference; and the failures (an element inverted by
crushing stops at the minimum increment, naming it; the caps; what is refused).
