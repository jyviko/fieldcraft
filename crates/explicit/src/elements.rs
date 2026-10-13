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
use modelcraft_model::{BulkViscosity, HourglassKind};

use crate::system::System;

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
    /// Nodal volumetric strain sums and the nodes' tetrahedron volumes.
    theta: Vec<f64>,
    tet_volume: Vec<f64>,
    tet_strain: Vec<[f64; 6]>,
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
            tet_strain: vec![[0.0; 6]; sys.tets.len()],
        }
    }

    /// The total internal force at a degree of freedom.
    pub fn internal(&self, dof: usize) -> f64 {
        let at = |v: &[f64]| v.get(dof).copied().unwrap_or(0.0);
        at(&self.stress_force) + at(&self.hourglass_force) + at(&self.viscous_force)
    }
}

fn gather<const N: usize>(x: &[f64], nodes: &[usize; N]) -> [[f64; 3]; N] {
    nodes.map(|n| [0, 1, 2].map(|k| x.get(3 * n + k).copied().unwrap_or(0.0)))
}

fn scatter<const N: usize>(f: &mut [f64], nodes: &[usize; N], v: &[[f64; 3]; N]) {
    for (n, vi) in nodes.iter().zip(v) {
        for (k, c) in vi.iter().enumerate() {
            if let Some(x) = f.get_mut(3 * n + k) {
                *x += c;
            }
        }
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

/// The forces of a uniform pressure q (compression positive).
fn pressure_stress(q: f64) -> [f64; 6] {
    [-q, -q, -q, 0.0, 0.0, 0.0]
}

/// Evaluate the internal forces at displacements `u` and velocities `v` into `ws` and return the
/// smallest critical increment of the elements.
pub fn internal_forces(sys: &System, u: &[f64], v: &[f64], bulk: &BulkViscosity, ws: &mut Workspace) -> f64 {
    for f in [&mut ws.stress_force, &mut ws.hourglass_force, &mut ws.viscous_force] {
        f.iter_mut().for_each(|x| *x = 0.0);
    }
    let mut critical = f64::INFINITY;
    for h in &sys.hexes {
        let Some(mat) = sys.materials.get(h.material) else { continue };
        let k = &h.kinematics;
        let (ue, ve) = (gather(u, &h.nodes), gather(v, &h.nodes));
        let strain = k.strain(&ue);
        let rate = k.strain(&ve);
        let sigma = stress(&mat.elasticity, &strain);
        let volumetric = rate[0] + rate[1] + rate[2];
        let q = bulk_pressure(volumetric, mat.density, h.length, mat.wave_speed, bulk);
        scatter(&mut ws.stress_force, &h.nodes, &k.forces(&sigma));
        if q != 0.0 {
            scatter(&mut ws.viscous_force, &h.nodes, &k.forces(&pressure_stress(q)));
        }
        let modes = match sys.hourglass {
            HourglassKind::Viscous => &ve,
            HourglassKind::Stiffness => &ue,
        };
        let mut f = [[0.0; 3]; 8];
        for g in &h.gamma {
            let generalized = [0, 1, 2].map(|i| h.hourglass * modes.iter().zip(g).map(|(x, gk)| x[i] * gk).sum::<f64>());
            for (fk, gk) in f.iter_mut().zip(g) {
                for i in 0..3 {
                    fk[i] += generalized[i] * gk;
                }
            }
        }
        scatter(&mut ws.hourglass_force, &h.nodes, &f);
        if let Some(s) = ws.stress.get_mut(h.index) {
            *s = sigma;
        }
        if let Some(s) = ws.strain.get_mut(h.index) {
            *s = strain;
        }
        critical = critical.min(critical_increment(volumetric, h.length, mat.wave_speed, bulk));
    }
    if !sys.tets.is_empty() {
        // Pass 1: each node's volume-weighted sum of its tetrahedra's volumetric strains.
        ws.theta.iter_mut().for_each(|x| *x = 0.0);
        for (t, saved) in sys.tets.iter().zip(ws.tet_strain.iter_mut()) {
            let strain = t.kinematics.strain(&gather(u, &t.nodes));
            for node in t.nodes {
                if let Some(x) = ws.theta.get_mut(node) {
                    *x += t.kinematics.volume * (strain[0] + strain[1] + strain[2]);
                }
            }
            *saved = strain;
        }
        // Pass 2: each tetrahedron's pressure from the mean of its nodes' volumetric strains.
        for (t, strain) in sys.tets.iter().zip(&ws.tet_strain) {
            let Some(mat) = sys.materials.get(t.material) else { continue };
            let k = &t.kinematics;
            let nodal = t.nodes.map(|n| ws.theta.get(n).copied().unwrap_or(0.0) / ws.tet_volume.get(n).copied().unwrap_or(1.0));
            let averaged = nodal.iter().sum::<f64>() / 4.0;
            let own = strain[0] + strain[1] + strain[2];
            let mut sigma = stress(&mat.elasticity, strain);
            let shift = mat.bulk.unwrap_or(0.0) * (averaged - own);
            for s in sigma.iter_mut().take(3) {
                *s += shift;
            }
            let rate = k.strain(&gather(v, &t.nodes));
            let volumetric = rate[0] + rate[1] + rate[2];
            let q = bulk_pressure(volumetric, mat.density, t.length, mat.wave_speed, bulk);
            scatter(&mut ws.stress_force, &t.nodes, &k.forces(&sigma));
            if q != 0.0 {
                scatter(&mut ws.viscous_force, &t.nodes, &k.forces(&pressure_stress(q)));
            }
            if let Some(s) = ws.stress.get_mut(t.index) {
                *s = sigma;
            }
            if let Some(s) = ws.strain.get_mut(t.index) {
                *s = *strain;
            }
            critical = critical.min(critical_increment(volumetric, t.length, mat.wave_speed, bulk));
        }
    }
    critical
}
