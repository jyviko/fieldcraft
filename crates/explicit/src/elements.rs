//! Element internal forces and critical time increments.
//!
//! **One-point hexahedron** (D. P. Flanagan, T. Belytschko, IJNME 17(5):679–706, 1981): the
//! stress of the mean strain gives the forces f_iI = σ_ij b_jI ([`modelcraft_fem::uniform`]).
//! The hourglass modes are resisted through the shape vectors γ_α: with the hourglass rates
//! q̇_iα = Σ_I v_iI γ_αI (or displacements q_iα = Σ_I u_iI γ_αI for the stiffness form), the
//! generalized forces Q_iα = C q̇_iα (viscous) or k q_iα (stiffness) give the nodal forces
//! f_iI = Σ_α Q_iα γ_αI. Because the γ_α are orthogonal to linear fields, these forces do no work
//! in a constant strain state. The coefficients are scaled from the dimensionless hourglass
//! coefficient κ with the element's size h = V / √(2 λ_max(Σ b bᵀ)) (the edge, for a cube),
//! density ρ and wave speed c: C = κ ρ c h² / 16 damps a cube's hourglass modes at a fraction κ
//! of critical at the frequency 2c/h, and k = κ ρ c² h / 16 puts their frequency at √κ · 2c/h.
//! (These normalizations are ours; the paper leaves the scale of the coefficient to the user.)
//!
//! **Average nodal pressure tetrahedron** (J. Bonet, A. J. Burton, *A simple average nodal
//! pressure tetrahedral element for incompressible and nearly incompressible dynamic explicit
//! applications*, Commun. Numer. Methods Eng. 14(5):437–449, 1998): the volumetric strain of each
//! node is the volume-weighted mean of its tetrahedra's, and each tetrahedron takes the mean of
//! its nodes' as its own: σ = 2μ dev ε + K θ̄ I. The forces are those of the strain energy
//! Σ_e V_e μ ‖dev ε_e‖² + ½ K Σ_a V_a θ_a², so the element conserves energy, and it does not lock
//! when the material is nearly incompressible.
//!
//! **Bulk viscosity** (J. von Neumann, R. D. Richtmyer, J. Appl. Phys. 21(3):232–237, 1950; M. L.
//! Wilkins, J. Comput. Phys. 36(3):281–303, 1980): in compression (volumetric strain rate
//! ε̇ᵥ < 0) a pressure q = ρ L (C_q L ε̇ᵥ² − C_l c ε̇ᵥ) spreads shocks over a few elements and
//! damps the ringing behind wave fronts. It lowers the stable increment to
//! Δt = L / (Q + √(Q² + c²)), Q = C_l c + C_q L |ε̇ᵥ| (Wilkins 1980).
//!
//! **Critical increment.** Each element's stable length L = 2c/ω_max comes from the bound on its
//! highest frequency ([`modelcraft_fem::uniform::UniformStrain::frequency_bound`], plus the
//! hourglass stiffness when it is used), so without bulk viscosity its critical increment is
//! 2/ω_max (T. Belytschko, W. K. Liu, B. Moran, K. Elkhodary, 2014, §6.3; the element eigenvalue
//! inequality makes the smallest of them stable for the mesh). For the nodal-pressure
//! tetrahedron the averaging only lowers the volumetric energy (the mean of squares bounds the
//! square of the mean), so the plain tetrahedron's bound holds.

use modelcraft_fem::material::stress;
use modelcraft_fem::uniform::UniformStrain;
use modelcraft_model::{BulkViscosity, HourglassKind};

use crate::system::{Material, System};

/// Scratch vectors and the element state of the latest evaluation.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Internal forces per degree of freedom, by origin.
    pub stress_force: Vec<f64>,
    pub hourglass_force: Vec<f64>,
    pub viscous_force: Vec<f64>,
    /// Per element (in [`System::elements`] order): the material stress and the strain.
    pub stress: Vec<[f64; 6]>,
    pub strain: Vec<[f64; 6]>,
    /// Per node: the mean volumetric strain of its tetrahedra (a volume-weighted sum until it is
    /// divided by `tet_volume`).
    theta: Vec<f64>,
    tet_volume: Vec<f64>,
}

impl Workspace {
    pub fn new(sys: &System) -> Workspace {
        let n = sys.labels.len();
        let mut tet_volume = vec![0.0; n];
        for t in &sys.tets {
            for node in t.nodes {
                if let Some(v) = tet_volume.get_mut(node) {
                    *v += t.kinematics.volume;
                }
            }
        }
        let dofs = sys.dofs();
        let elements = sys.elements.len();
        Workspace {
            stress_force: vec![0.0; dofs],
            hourglass_force: vec![0.0; dofs],
            viscous_force: vec![0.0; dofs],
            stress: vec![[0.0; 6]; elements],
            strain: vec![[0.0; 6]; elements],
            theta: vec![0.0; n],
            tet_volume,
        }
    }
}

pub(crate) fn gather<const N: usize>(x: &[f64], nodes: &[usize; N]) -> [[f64; 3]; N] {
    nodes.map(|n| [0, 1, 2].map(|k| x.get(3 * n + k).copied().unwrap_or(0.0)))
}

/// Add a node's vector into a per-degree-of-freedom vector.
pub(crate) fn add_at(f: &mut [f64], node: usize, v: &[f64; 3]) {
    for (k, c) in v.iter().enumerate() {
        if let Some(x) = f.get_mut(3 * node + k) {
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

/// What the hexahedron and the tetrahedron share once their stress is known: the stress and bulk
/// viscosity forces, the element state and the critical increment.
struct Finish<'a, const N: usize> {
    kinematics: &'a UniformStrain<N>,
    nodes: &'a [usize; N],
    index: usize,
    length: f64,
}

impl<const N: usize> Finish<'_, N> {
    fn apply(&self, ws: &mut Workspace, mat: &Material, v: &[f64], sigma: [f64; 6], strain: [f64; 6], bulk: &BulkViscosity) -> f64 {
        let k = self.kinematics;
        let rate = k.divergence(&gather(v, self.nodes));
        let q = bulk_pressure(rate, mat.density, self.length, mat.wave_speed, bulk);
        scatter(&mut ws.stress_force, self.nodes, &k.forces(&sigma));
        if q != 0.0 {
            scatter(&mut ws.viscous_force, self.nodes, &k.pressure_forces(q));
        }
        if let Some(s) = ws.stress.get_mut(self.index) {
            *s = sigma;
        }
        if let Some(s) = ws.strain.get_mut(self.index) {
            *s = strain;
        }
        critical_increment(rate, self.length, mat.wave_speed, bulk)
    }
}

/// Evaluate the internal forces at displacements `u` and velocities `v` into `ws` and return the
/// smallest critical increment of the elements.
pub fn internal_forces(sys: &System, u: &[f64], v: &[f64], bulk: &BulkViscosity, ws: &mut Workspace) -> f64 {
    for f in [&mut ws.stress_force, &mut ws.hourglass_force, &mut ws.viscous_force] {
        f.fill(0.0);
    }
    let mut critical = f64::INFINITY;
    for h in &sys.hexes {
        let Some(mat) = sys.materials.get(h.material) else { continue };
        let ue = gather(u, &h.nodes);
        let strain = h.kinematics.strain(&ue);
        let sigma = stress(&mat.elasticity, &strain);
        // Hourglass forces on the displacement (stiffness) or velocity (viscous) modes.
        let modes = match sys.hourglass {
            HourglassKind::Viscous => gather(v, &h.nodes),
            HourglassKind::Stiffness => ue,
        };
        let mut f = [[0.0; 3]; 8];
        for g in &h.gamma {
            let mut generalized = [0.0; 3];
            for (x, gk) in modes.iter().zip(g) {
                for i in 0..3 {
                    generalized[i] += x[i] * gk;
                }
            }
            for (fk, gk) in f.iter_mut().zip(g) {
                for i in 0..3 {
                    fk[i] += h.hourglass * generalized[i] * gk;
                }
            }
        }
        scatter(&mut ws.hourglass_force, &h.nodes, &f);
        let finish = Finish { kinematics: &h.kinematics, nodes: &h.nodes, index: h.index, length: h.length };
        critical = critical.min(finish.apply(ws, mat, v, sigma, strain, bulk));
    }
    if !sys.tets.is_empty() {
        // Pass 1: each tetrahedron's strain, and each node's mean volumetric strain.
        ws.theta.fill(0.0);
        for t in &sys.tets {
            let strain = t.kinematics.strain(&gather(u, &t.nodes));
            for node in t.nodes {
                if let Some(x) = ws.theta.get_mut(node) {
                    *x += t.kinematics.volume * (strain[0] + strain[1] + strain[2]);
                }
            }
            if let Some(s) = ws.strain.get_mut(t.index) {
                *s = strain;
            }
        }
        for (x, v) in ws.theta.iter_mut().zip(&ws.tet_volume) {
            if *v > 0.0 {
                *x /= v;
            }
        }
        // Pass 2: each tetrahedron's pressure from the mean of its nodes' volumetric strains.
        for t in &sys.tets {
            let Some(mat) = sys.materials.get(t.material) else { continue };
            let strain = ws.strain.get(t.index).copied().unwrap_or_default();
            let averaged = t.nodes.iter().map(|n| ws.theta.get(*n).copied().unwrap_or(0.0)).sum::<f64>() / 4.0;
            let own = strain[0] + strain[1] + strain[2];
            let mut sigma = stress(&mat.elasticity, &strain);
            let shift = mat.bulk.unwrap_or(0.0) * (averaged - own);
            for s in sigma.iter_mut().take(3) {
                *s += shift;
            }
            let finish = Finish { kinematics: &t.kinematics, nodes: &t.nodes, index: t.index, length: t.length };
            critical = critical.min(finish.apply(ws, mat, v, sigma, strain, bulk));
        }
    }
    critical
}
