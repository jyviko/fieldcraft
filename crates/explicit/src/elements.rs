//! Element internal forces, the material update and critical time increments.
//!
//! **Kinematics.** With nonlinear geometry the elements are updated Lagrangian (T. Belytschko,
//! W. K. Liu, B. Moran, K. Elkhodary, *Nonlinear Finite Elements for Continua and Structures*,
//! 2nd ed., Wiley 2014, ch. 4 and 6): the strain and spin increments of an increment come from
//! the displacement increment Δu and the gradients of the **midpoint** configuration
//! x_{n−½} = X + u_n − ½Δu, ΔG = (1/V) Σ_I Δu_I ⊗ b_I(x_{n−½}), which makes the update
//! incrementally objective (T. J. R. Hughes, J. Winget, IJNME 15(12):1862–1867, 1980); the nodal
//! forces of the Cauchy stress use the gradients of the current configuration,
//! f_iI = σ_ij b_jI(x_n). Without it, both are the gradients of the undeformed mesh and the
//! update is small-strain elasticity (and plasticity).
//!
//! **Stress update** in rate form: the material state (stress, back stress, the accumulated
//! strain and the stiffness-form hourglass forces) is rotated with the increment's rotation —
//! the Cayley transform of the spin increment (Jaumann rate, Hughes & Winget 1980) or the
//! increment of the polar rotation (Green–Naghdi rate, D. P. Flanagan, L. M. Taylor, CMAME
//! 62(3):305–320, 1987), see [`modelcraft_fem::rotation`] — then the constitutive increment is
//! added: hypoelastic, or J2 radial return with Johnson–Cook damage
//! ([`modelcraft_fem::plasticity`]). Orthotropic elasticity is updated in the polar frame (its
//! axes turn with the material) whichever rate the step names.
//!
//! **One-point hexahedron** (D. P. Flanagan, T. Belytschko, IJNME 17(5):679–706, 1981): the
//! hourglass modes are resisted through the shape vectors γ_α of the current configuration:
//! with the hourglass rates q̇_iα = Σ_I v_iI γ_αI, the generalized forces are Q_iα = C q̇_iα
//! (viscous) or accumulate as Q_iα += k Σ_I Δu_iI γ_αI (stiffness, rotated with the element),
//! and give the nodal forces f_iI = Σ_α Q_iα γ_αI. Because the γ_α are orthogonal to linear
//! fields, these forces do no work in a constant strain state. The coefficients are scaled from
//! the dimensionless hourglass coefficient κ with the element's current size
//! h = V / √(2 λ_max(Σ b bᵀ)) (the edge, for a cube), density ρ and wave speed c:
//! C = κ ρ c h² / 16 damps a cube's hourglass modes at a fraction κ of critical at the frequency
//! 2c/h, and k = κ ρ c² h / 16 puts their frequency at √κ · 2c/h. (These normalizations are ours;
//! the paper leaves the scale of the coefficient to the user.)
//!
//! **Average nodal pressure tetrahedron** (J. Bonet, A. J. Burton, Commun. Numer. Methods Eng.
//! 14(5):437–449, 1998), in rate form: the volumetric strain increment of each node is the
//! volume-weighted mean of its tetrahedra's, and each tetrahedron takes the mean of its nodes' as
//! its own, so its pressure increment is K times that mean while the deviator is its own. It does
//! not lock when the material is nearly incompressible or flows plastically.
//!
//! **Bulk viscosity** (J. von Neumann, R. D. Richtmyer, J. Appl. Phys. 21(3):232–237, 1950; M. L.
//! Wilkins, J. Comput. Phys. 36(3):281–303, 1980): in compression (volumetric strain rate
//! ε̇ᵥ < 0) a pressure q = ρ L (C_q L ε̇ᵥ² − C_l c ε̇ᵥ) spreads shocks over a few elements and
//! damps the ringing behind wave fronts. It lowers the stable increment to
//! Δt = L / (Q + √(Q² + c²)), Q = C_l c + C_q L |ε̇ᵥ| (Wilkins 1980).
//!
//! **Critical increment.** Each element's stable length L = 2c/ω_max comes from the bound on its
//! highest frequency in its current shape ([`modelcraft_fem::uniform`], with the element's lumped
//! masses, plus the hourglass stiffness when it is used), so without bulk viscosity its critical
//! increment is 2/ω_max (Belytschko et al. 2014, §6.3). The elastic moduli bound the tangent
//! moduli of plastic flow, so the bound holds for the inelastic materials too.
//!
//! **Erosion** (G. R. Johnson, R. A. Stryk, Int. J. Impact Eng. 5:411–421, 1987; T. Belytschko,
//! J. I. Lin, ibid. 5:111–127, 1987): an element whose material point fails (damage 1) or
//! reaches its material's erosion plastic strain is removed: it carries no stress from then on,
//! its nodes keep their mass and momentum, and its internal energy at that moment is reported.

use modelcraft_fem::material::stress;
use modelcraft_fem::plasticity::{Increment, PointState};
use modelcraft_fem::rotation::{self, IDENTITY, Mat3, Polar};
use modelcraft_fem::uniform::{Hex8, Tet4, UniformStrain};
use modelcraft_model::{BulkViscosity, HourglassKind, StressRate};

use crate::system::{Material, NODE_DOFS, System};
use crate::{Error, Result};

/// Scratch vectors of one evaluation.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Internal forces per degree of freedom, by origin.
    pub stress_force: Vec<f64>,
    pub hourglass_force: Vec<f64>,
    pub viscous_force: Vec<f64>,
    /// Per node: the volume-weighted volumetric strain increments of its tetrahedra, and their
    /// volume.
    theta: Vec<f64>,
    tet_volume: Vec<f64>,
    /// Per tetrahedron (in [`System::tets`] order): its increment between the two passes.
    tet_increment: Vec<Option<TetIncrement>>,
}

impl Workspace {
    pub fn new(sys: &System) -> Workspace {
        let n = sys.labels.len();
        let dofs = sys.dofs();
        Workspace {
            stress_force: vec![0.0; dofs],
            hourglass_force: vec![0.0; dofs],
            viscous_force: vec![0.0; dofs],
            theta: vec![0.0; n],
            tet_volume: vec![0.0; n],
            tet_increment: vec![None; sys.tets.len()],
        }
    }
}

/// Why an element was eroded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// Its damage reached 1.
    Damage,
    /// Its equivalent plastic strain reached the material's erosion strain.
    PlasticStrain,
}

/// The state of one element (its single material point).
#[derive(Debug, Clone)]
pub struct ElementState {
    pub point: PointState,
    /// The accumulated (rotated) strain increments: engineering shears.
    pub strain: [f64; 6],
    /// The polar rotation and stretch (Green–Naghdi rate and orthotropic materials).
    pub polar: Polar,
    /// Stiffness-form hourglass generalized forces Q_α (one-point hexahedra).
    pub hourglass: [[f64; 3]; 4],
    /// Work of the stress on the element (its internal energy, hourglass apart).
    pub energy: f64,
    /// Plastic work on the element.
    pub plastic_work: f64,
    pub failed: Option<Failure>,
}

/// The element states and the erosions of a run.
#[derive(Debug, Clone)]
pub struct State {
    /// Per element, in [`System::elements`] order.
    pub elements: Vec<ElementState>,
    /// Elements eroded during the latest evaluation: (index, failure).
    pub eroded: Vec<(usize, Failure)>,
}

impl State {
    pub fn new(sys: &System) -> State {
        let mut material = vec![usize::MAX; sys.elements.len()];
        for (index, m) in sys.hexes.iter().map(|h| (h.index, h.material)).chain(sys.tets.iter().map(|t| (t.index, t.material))) {
            if let Some(slot) = material.get_mut(index) {
                *slot = m;
            }
        }
        let elements = material
            .into_iter()
            .map(|m| {
                let temperature = sys.materials.get(m).and_then(|m| m.j2.as_ref()).map_or(0.0, |j| j.initial_temperature());
                ElementState {
                    point: PointState::new(temperature),
                    strain: [0.0; 6],
                    polar: Polar::default(),
                    hourglass: [[0.0; 3]; 4],
                    energy: 0.0,
                    plastic_work: 0.0,
                    failed: None,
                }
            })
            .collect();
        State { elements, eroded: Vec::new() }
    }
}

pub(crate) fn gather<const N: usize>(x: &[f64], nodes: &[usize; N]) -> [[f64; 3]; N] {
    nodes.map(|n| [0, 1, 2].map(|k| x.get(NODE_DOFS * n + k).copied().unwrap_or(0.0)))
}

/// Add a node's vector into a per-degree-of-freedom vector.
pub(crate) fn add_at(f: &mut [f64], node: usize, v: &[f64; 3]) {
    for (k, c) in v.iter().enumerate() {
        if let Some(x) = f.get_mut(NODE_DOFS * node + k) {
            *x += c;
        }
    }
}

fn scatter<const N: usize>(f: &mut [f64], nodes: &[usize; N], v: &[[f64; 3]; N]) {
    for (n, vi) in nodes.iter().zip(v) {
        add_at(f, *n, vi);
    }
}

/// The bulk viscosity pressure (positive in compression) for a volumetric strain rate.
pub fn bulk_pressure(rate: f64, density: f64, length: f64, wave_speed: f64, b: &BulkViscosity) -> f64 {
    if rate < 0.0 { density * length * (b.quadratic * length * rate * rate - b.linear * wave_speed * rate) } else { 0.0 }
}

/// The critical increment of an element of stable length `length` at a volumetric strain rate
/// (Wilkins 1980).
pub fn critical_increment(rate: f64, length: f64, wave_speed: f64, b: &BulkViscosity) -> f64 {
    let q = if rate < 0.0 { b.linear * wave_speed + b.quadratic * length * rate.abs() } else { 0.0 };
    length / (q + (q * q + wave_speed * wave_speed).sqrt())
}

/// The largest absolute row sum of the hourglass vectors' Gram matrix Σ_I γ_αI γ_βI / m_I (an
/// upper bound of its largest eigenvalue, exact for parallelepipeds where the γ_α are
/// orthogonal).
pub fn hourglass_gram_bound(gamma: &[[f64; 8]; 4], mass: &[f64; 8]) -> f64 {
    let mut worst: f64 = 0.0;
    for a in gamma {
        let row: f64 = gamma.iter().map(|b| a.iter().zip(b).zip(mass).map(|((x, y), m)| x * y / m).sum::<f64>().abs()).sum();
        worst = worst.max(row);
    }
    worst
}

/// What one evaluation of the internal forces is given.
pub struct Motion<'a> {
    /// Displacements u^n.
    pub u: &'a [f64],
    /// The displacement increment that led to `u`, Δt^{n−½} v^{n−½}.
    pub du: &'a [f64],
    /// The velocity v^{n−½}.
    pub v: &'a [f64],
    /// The increment's length Δt^{n−½} (0 at the start).
    pub dt: f64,
    pub time: f64,
    pub bulk: &'a BulkViscosity,
}

/// The element's nodal values: rest positions, displacements, displacement increments and
/// velocities, gathered once per evaluation.
struct Nodal<const N: usize> {
    x: [[f64; 3]; N],
    u: [[f64; 3]; N],
    du: [[f64; 3]; N],
    v: [[f64; 3]; N],
}

impl<const N: usize> Nodal<N> {
    fn gather(sys: &System, nodes: &[usize; N], m: &Motion) -> Nodal<N> {
        Nodal { x: nodes.map(|n| sys.xyz.get(n).copied().unwrap_or_default()), u: gather(m.u, nodes), du: gather(m.du, nodes), v: gather(m.v, nodes) }
    }

    /// The positions X + u − s·Δu.
    fn positions(&self, s: f64) -> [[f64; 3]; N] {
        std::array::from_fn(|i| [0, 1, 2].map(|k| self.x[i][k] + self.u[i][k] - s * self.du[i][k]))
    }
}

fn inverted(label: u32, time: f64) -> Error {
    Error::Unstable(format!("element {label} turned inside out at time {time:.6e} (its volume is no longer positive)"))
}

/// The rotation of an increment for an element's state, advancing its polar decomposition when
/// it is tracked.
fn rotation_of(sys: &System, mat: &Material, es: &mut ElementState, g: &Mat3, label: u32, time: f64) -> Result<Mat3> {
    if !sys.nonlinear {
        return Ok(IDENTITY);
    }
    if sys.stress_rate == StressRate::GreenNaghdi || mat.j2.is_none() {
        es.polar.advance(g).ok_or_else(|| inverted(label, time))
    } else {
        let (_, w) = rotation::split(g);
        rotation::cayley(&w).ok_or_else(|| inverted(label, time))
    }
}

/// Rotate an element's state with `q`, then add the constitutive increment for the strain
/// increment `d` (engineering shears) with volumetric part `vol` (the tetrahedron's averaged
/// one; the trace of `d` for the hexahedron), track its energy at the midpoint volume `volume`,
/// and check for erosion.
#[allow(clippy::too_many_arguments)]
fn update(mat: &Material, es: &mut ElementState, q: &Mat3, d: &[f64; 6], vol: f64, dt: f64, density: f64, volume: f64) {
    let p = &mut es.point;
    if *q != IDENTITY {
        // Only the parts of the state that are in use (a back stress with kinematic hardening,
        // hourglass forces with stiffness-form control).
        p.stress = rotation::rotate(q, &p.stress);
        if p.back != [0.0; 6] {
            p.back = rotation::rotate(q, &p.back);
        }
        es.strain = rotation::rotate_strain(q, &es.strain);
        if es.hourglass != [[0.0; 3]; 4] {
            for h in &mut es.hourglass {
                *h = rotation::apply(q, h);
            }
        }
    }
    let before = p.stress;
    let inc = match &mat.j2 {
        Some(j2) => j2.update(p, d, vol, dt, density),
        None => {
            // Orthotropic: in the polar frame, σ̂ += D d̂.
            let r = es.polar.rotation;
            if r == IDENTITY {
                for (s, x) in p.stress.iter_mut().zip(stress(&mat.elasticity, d)) {
                    *s += x;
                }
            } else {
                let local = rotation::rotate_strain(&rotation::transpose(&r), d);
                for (s, x) in p.stress.iter_mut().zip(rotation::rotate(&r, &stress(&mat.elasticity, &local))) {
                    *s += x;
                }
            }
            Increment::default()
        }
    };
    // Stress work over the increment, ½(σ_n + σ_{n+1}) : Δε, with the volumetric strain the
    // stress took.
    let shift = (d[0] + d[1] + d[2] - vol) / 3.0;
    let mut work = 0.0;
    for k in 0..6 {
        let dk = if k < 3 { d[k] - shift } else { d[k] };
        work += 0.5 * (before[k] + p.stress[k]) * dk;
        es.strain[k] += dk;
    }
    es.energy += work * volume;
    es.plastic_work += inc.plastic_work * volume;
    if es.failed.is_none() {
        if p.damage >= 1.0 {
            es.failed = Some(Failure::Damage);
        } else if mat.erosion.is_some_and(|e| p.plastic_strain >= e) {
            es.failed = Some(Failure::PlasticStrain);
        }
    }
}

/// The density and dilatational wave speed of an element of rest volume `rest`, now of volume
/// `volume`.
fn current(mat: &Material, rest: f64, volume: f64) -> (f64, f64) {
    let rho = mat.density * rest / volume;
    (rho, (mat.stiffest / rho).sqrt())
}

/// The bound ω² ≤ k λ_max(Σ b bᵀ) / (V m_min) on an element's highest frequency
/// ([`modelcraft_fem::uniform`]), with the lumped masses of the undeformed element and
/// `gradient_norm` = λ_max(Σ b bᵀ).
pub fn frequency_bound<const N: usize>(k: &UniformStrain<N>, gradient_norm: f64, modulus: f64, mass: &[f64; N]) -> f64 {
    let lightest = mass.iter().copied().fold(f64::INFINITY, f64::min);
    modulus * gradient_norm / (k.volume * lightest)
}

/// A hexahedron's size h, hourglass coefficient (damping or stiffness) and frequency bound ω²
/// (with the hourglass stiffness), at density `density`.
#[derive(Debug, Clone, Copy)]
pub struct HexScales {
    pub size: f64,
    pub coefficient: f64,
    pub omega2: f64,
}

/// The [`HexScales`] of a hexahedron in configuration `k` with shape vectors `gamma`.
pub fn hex_scales(
    k: &Hex8,
    gamma: &[[f64; 8]; 4],
    mass: &[f64; 8],
    mat: &Material,
    density: f64,
    hourglass: modelcraft_model::Hourglass,
) -> HexScales {
    let norm = k.gradient_norm();
    // h: the edge of the cube with the element's volume and gradient norm.
    let size = k.volume / (2.0 * norm).sqrt();
    let wave2 = mat.stiffest / density;
    let mut omega2 = frequency_bound(k, norm, mat.modulus, mass);
    let kappa = hourglass.coefficient;
    let coefficient = match hourglass.kind {
        HourglassKind::Viscous => kappa * density * wave2.sqrt() * size * size / 16.0,
        HourglassKind::Stiffness => {
            let stiffness = kappa * density * wave2 * size / 16.0;
            omega2 += stiffness * hourglass_gram_bound(gamma, mass);
            stiffness
        }
    };
    HexScales { size, coefficient, omega2 }
}

/// Evaluate the internal forces at the motion `m` into `ws`, updating the element states, and
/// return the smallest critical increment of the elements still in the analysis.
pub fn internal_forces(sys: &System, m: &Motion, state: &mut State, ws: &mut Workspace) -> Result<f64> {
    for f in [&mut ws.stress_force, &mut ws.hourglass_force, &mut ws.viscous_force] {
        f.fill(0.0);
    }
    state.eroded.clear();
    let mut critical = f64::INFINITY;
    let hourglass = modelcraft_model::Hourglass { kind: sys.hourglass, coefficient: sys.hourglass_coefficient };
    for h in &sys.hexes {
        let (Some(mat), Some(es)) = (sys.materials.get(h.material), state.elements.get_mut(h.index)) else { continue };
        if es.failed.is_some() {
            continue;
        }
        let e = Nodal::gather(sys, &h.nodes, m);
        let x_now = e.positions(0.0);
        let (mid, now) = if sys.nonlinear {
            let at = |x: &[[f64; 3]; 8]| Hex8::hex8_deformed(x).map_err(|_| inverted(h.label, m.time));
            (at(&e.positions(0.5))?, at(&x_now)?)
        } else {
            (h.kinematics, h.kinematics)
        };
        let g = mid.gradient(&e.du);
        let q = rotation_of(sys, mat, es, &g, h.label, m.time)?;
        let (d, _) = rotation::split(&g);
        let (rho, c) = current(mat, h.kinematics.volume, now.volume);
        update(mat, es, &q, &d, d[0] + d[1] + d[2], m.dt, rho, mid.volume);
        if let Some(failure) = es.failed {
            state.eroded.push((h.index, failure));
            continue;
        }
        // Hourglass forces with the current shape vectors and size (fixed without nonlinear
        // geometry).
        let (gamma, scales) = if sys.nonlinear {
            let gamma = now.hourglass_vectors(&x_now);
            let scales = hex_scales(&now, &gamma, &h.mass, mat, rho, hourglass);
            (gamma, scales)
        } else {
            (h.gamma, h.rest)
        };
        let mut f = [[0.0; 3]; 8];
        let modes = if sys.hourglass == HourglassKind::Viscous { &e.v } else { &e.du };
        for (k, g) in gamma.iter().enumerate() {
            let rate: [f64; 3] = [0, 1, 2].map(|i| modes.iter().zip(g).map(|(x, gk)| x[i] * gk).sum());
            // Viscous: Q = C q̇; stiffness: Q accumulates k Δq.
            let generalized = match (sys.hourglass, es.hourglass.get_mut(k)) {
                (HourglassKind::Stiffness, Some(qa)) => {
                    for i in 0..3 {
                        qa[i] += scales.coefficient * rate[i];
                    }
                    *qa
                }
                _ => rate.map(|r| scales.coefficient * r),
            };
            for (fk, gk) in f.iter_mut().zip(g) {
                for i in 0..3 {
                    fk[i] += generalized[i] * gk;
                }
            }
        }
        scatter(&mut ws.hourglass_force, &h.nodes, &f);
        let element = Element { now: &now, mid: &mid, nodes: &h.nodes, density: rho, wave_speed: c, length: 2.0 * c / scales.omega2.sqrt() };
        critical = critical.min(element.finish(ws, &es.point.stress, &e.v, m.bulk));
    }
    if !sys.tets.is_empty() {
        // Pass 1: each tetrahedron's increment, and each node's mean volumetric increment.
        ws.theta.fill(0.0);
        ws.tet_volume.fill(0.0);
        for (k, t) in sys.tets.iter().enumerate() {
            let Some(slot) = ws.tet_increment.get_mut(k) else { continue };
            *slot = None;
            if state.elements.get(t.index).is_none_or(|e| e.failed.is_some()) {
                continue;
            }
            let e = Nodal::gather(sys, &t.nodes, m);
            let (mid, now) = if sys.nonlinear {
                let at = |x: &[[f64; 3]; 4]| Tet4::tet4(x).map_err(|_| inverted(t.label, m.time));
                (at(&e.positions(0.5))?, at(&e.positions(0.0))?)
            } else {
                (t.kinematics, t.kinematics)
            };
            let g = mid.gradient(&e.du);
            for node in t.nodes {
                if let (Some(x), Some(w)) = (ws.theta.get_mut(node), ws.tet_volume.get_mut(node)) {
                    *x += mid.volume * (g[0][0] + g[1][1] + g[2][2]);
                    *w += mid.volume;
                }
            }
            *slot = Some(TetIncrement { g, mid, now, v: e.v });
        }
        for (x, w) in ws.theta.iter_mut().zip(&ws.tet_volume) {
            if *w > 0.0 {
                *x /= w;
            }
        }
        // Pass 2: each tetrahedron's update with the mean of its nodes' volumetric increments.
        for (k, t) in sys.tets.iter().enumerate() {
            let (Some(mat), Some(es), Some(Some(inc))) =
                (sys.materials.get(t.material), state.elements.get_mut(t.index), ws.tet_increment.get(k).copied())
            else {
                continue;
            };
            let averaged = t.nodes.iter().map(|n| ws.theta.get(*n).copied().unwrap_or(0.0)).sum::<f64>() / 4.0;
            let q = rotation_of(sys, mat, es, &inc.g, t.label, m.time)?;
            let (d, _) = rotation::split(&inc.g);
            let (rho, c) = current(mat, t.kinematics.volume, inc.now.volume);
            update(mat, es, &q, &d, averaged, m.dt, rho, inc.mid.volume);
            if let Some(failure) = es.failed {
                state.eroded.push((t.index, failure));
                continue;
            }
            let omega2 = if sys.nonlinear { frequency_bound(&inc.now, inc.now.gradient_norm(), mat.modulus, &t.mass) } else { t.omega2 };
            let element = Element { now: &inc.now, mid: &inc.mid, nodes: &t.nodes, density: rho, wave_speed: c, length: 2.0 * c / omega2.sqrt() };
            critical = critical.min(element.finish(ws, &es.point.stress, &inc.v, m.bulk));
        }
    }
    for r in &sys.rods {
        let (sigma, strain) = r.forces(&sys.xyz, m.u, sys.nonlinear, &mut ws.stress_force);
        if let Some(es) = state.elements.get_mut(r.index) {
            es.point.stress = sigma;
            es.strain = strain;
        }
        if let Some(mat) = sys.materials.get(r.material) {
            critical = critical.min(r.length / mat.wave_speed);
        }
    }
    for l in &sys.discrete.links {
        l.forces(&sys.xyz, m.u, m.v, sys.nonlinear, &mut ws.stress_force, &mut ws.viscous_force);
    }
    if let Some((dt, _)) = &sys.discrete_increment {
        critical = critical.min(*dt);
    }
    Ok(critical)
}

/// A tetrahedron's increment between the two passes: the gradient of its displacement increment,
/// its midpoint and current kinematics, and its nodal velocities.
#[derive(Debug, Clone, Copy)]
struct TetIncrement {
    g: Mat3,
    mid: Tet4,
    now: Tet4,
    v: [[f64; 3]; 4],
}

/// What the hexahedron and the tetrahedron share once their stress is known.
struct Element<'a, const N: usize> {
    now: &'a UniformStrain<N>,
    mid: &'a UniformStrain<N>,
    nodes: &'a [usize; N],
    density: f64,
    wave_speed: f64,
    length: f64,
}

impl<const N: usize> Element<'_, N> {
    /// Add the stress and bulk viscosity forces and return the critical increment.
    fn finish(&self, ws: &mut Workspace, sigma: &[f64; 6], v: &[[f64; 3]; N], bulk: &BulkViscosity) -> f64 {
        let rate = self.mid.divergence(v);
        let q = bulk_pressure(rate, self.density, self.length, self.wave_speed, bulk);
        scatter(&mut ws.stress_force, self.nodes, &self.now.forces(sigma));
        if q != 0.0 {
            scatter(&mut ws.viscous_force, self.nodes, &self.now.pressure_forces(q));
        }
        critical_increment(rate, self.length, self.wave_speed, bulk)
    }
}
