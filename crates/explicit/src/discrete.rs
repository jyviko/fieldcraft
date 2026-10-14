//! Trusses, point masses, springs and dashpots in explicit dynamics.
//!
//! - **Trusses** (rod2, rod3), elastic, with lumped (HRZ) mass. With nonlinear geometry a rod2
//!   is corotational: its strain is (l − L)/L from its current length l and its force acts along
//!   its current axis (Belytschko et al. 2014, §2.4 and §4.5); a rod3 then is not supported yet.
//!   Without it the internal force is K u with the kernel's stiffness ([`fieldcraft_fem::truss`]).
//!   The stress is the uniaxial tensor along the axis (the mean of the integration points'), and
//!   the critical increment comes from the kernel's frequency bound (exact for rod2: L/c).
//! - **Point masses** add to the lumped mass of their node, per direction.
//! - **Springs** add the force k (cᵀ Δu) c and **dashpots** c_d (cᵀ Δv) c, with Δu, Δv the
//!   relative motion of the ends along the unit direction c (a fixed direction; one along the line
//!   of the ends follows that line as it turns under nonlinear geometry, with the stretch
//!   l − L₀); dashpots use the velocity of the
//!   previous half increment, v^{n−½}, as the bulk viscosity does (Belytschko, Liu, Moran,
//!   Elkhodary 2014, §6.2). Spring work counts as internal energy, dashpot work as viscous
//!   (dissipated) energy.
//! - **Stable increment** of springs and dashpots, per degree of freedom: with
//!   ω² ≤ Σⱼ |Kᵢⱼ| / √(mᵢ mⱼ) (Gershgorin's bound on M^{-1/2} K M^{-1/2}, exact for one spring on
//!   one mass) and the same row sum D of the damping, the central-difference scheme with lagged
//!   damping is stable for Δt ≤ (2/ω)(√(1 + ξ²) − ξ) with ξ = D / (2ω) (Belytschko et al. 2014,
//!   §6.3), which is 2/D for a dashpot alone. Rotary inertia and rotational springs need
//!   rotational degrees of freedom, which this solver does not have yet.

use fieldcraft_fem::DofKind;
use fieldcraft_fem::dense::Matrix;
use fieldcraft_fem::truss::{Truss, uniaxial};
use fieldcraft_model::{ElementKind, Resolved, ResolvedDirection};

use crate::system::NODE_DOFS;
use crate::{Error, Result};

/// A truss element.
#[derive(Debug, Clone)]
pub struct Rod {
    pub label: u32,
    /// Position in [`crate::system::System::elements`].
    pub index: usize,
    pub nodes: Vec<usize>,
    pub material: usize,
    pub young: f64,
    /// Stiffness on the element's translations, node by node.
    pub stiffness: Matrix,
    /// Per integration point: the axial strain operator and the tangent.
    pub strain: Vec<(Vec<f64>, [f64; 3])>,
    /// The lumped mass per node for unit density.
    pub nodal_volume: Vec<f64>,
    /// The stable length: the critical increment is `length / wave_speed`.
    pub length: f64,
    /// E A and the rest length (a rod2's corotational force).
    pub axial: f64,
    pub rest_length: f64,
}

/// What a truss is made of: Young's modulus, density and cross-sectional area.
#[derive(Debug, Clone, Copy)]
pub struct RodSection {
    pub young: f64,
    pub density: f64,
    pub area: f64,
}

impl Rod {
    /// `material` is the index in [`crate::system::System::materials`].
    pub fn new(
        label: u32,
        index: usize,
        kind: ElementKind,
        nodes: Vec<usize>,
        xyz: &[[f64; 3]],
        material: usize,
        section: RodSection,
    ) -> Result<Rod> {
        let RodSection { young, density, area } = section;
        let at = |e| Error::element(label, e);
        let t = Truss::new(kind, xyz).map_err(at)?;
        let omega2 = t.frequency_bound(young, density).map_err(at)?;
        let wave_speed = (young / density).sqrt();
        Ok(Rod {
            label,
            index,
            nodes,
            material,
            young,
            stiffness: t.stiffness(young, area).map_err(at)?,
            strain: t.strain_operator().map_err(at)?,
            nodal_volume: t.lumped_mass(1.0, area).map_err(at)?,
            length: 2.0 * wave_speed / omega2.sqrt(),
            axial: young * area,
            rest_length: t.length().map_err(at)?,
        })
    }

    /// The element's degrees of freedom, node by node.
    pub fn dofs(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes.iter().flat_map(|n| fieldcraft_fem::truss::DOFS.node.iter().filter_map(DofKind::direction).map(|d| dof(*n, d)))
    }

    /// Add its internal force into `force` and return the element's mean stress and strain
    /// tensors: corotational for a rod2 with nonlinear geometry (`xyz` the rest positions of the
    /// system's nodes), K u otherwise.
    pub fn forces(&self, xyz: &[[f64; 3]], u: &[f64], nonlinear: bool, force: &mut [f64]) -> ([f64; 6], [f64; 6]) {
        if nonlinear && let [a, b] = self.nodes[..] {
            let at = |n: usize| -> [f64; 3] {
                let x = xyz.get(n).copied().unwrap_or_default();
                [0, 1, 2].map(|k| x[k] + u.get(dof(n, k)).copied().unwrap_or(0.0))
            };
            let (pa, pb) = (at(a), at(b));
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let t = if l > 0.0 { d.map(|c| c / l) } else { [0.0; 3] };
            let e = (l - self.rest_length) / self.rest_length;
            let n = self.axial * e;
            for k in 0..3 {
                if let Some(x) = force.get_mut(dof(a, k)) {
                    *x -= n * t[k];
                }
                if let Some(x) = force.get_mut(dof(b, k)) {
                    *x += n * t[k];
                }
            }
            return (uniaxial(self.young * e, t, 1.0), uniaxial(e, t, 2.0));
        }
        // At most three nodes of three translations: the element vectors live on the stack.
        let n = self.stiffness.n.min(MAX_DOFS);
        let mut ue = [0.0; MAX_DOFS];
        for (x, d) in ue.iter_mut().zip(self.dofs()) {
            *x = u.get(d).copied().unwrap_or(0.0);
        }
        for (i, d) in self.dofs().enumerate().take(n) {
            let f: f64 = (0..n).map(|j| self.stiffness.get(i, j) * ue[j]).sum();
            if let Some(x) = force.get_mut(d) {
                *x += f;
            }
        }
        let (mut stress, mut strain) = ([0.0; 6], [0.0; 6]);
        let share = 1.0 / self.strain.len().max(1) as f64;
        for (row, t) in &self.strain {
            let e: f64 = row.iter().zip(&ue).map(|(a, b)| a * b).sum();
            for (k, (s, x)) in uniaxial(self.young * e, *t, 1.0).iter().zip(uniaxial(e, *t, 2.0)).enumerate() {
                stress[k] += share * s;
                strain[k] += share * x;
            }
        }
        (stress, strain)
    }
}

/// Degrees of freedom of the largest truss (rod3: three nodes of three translations).
const MAX_DOFS: usize = 9;

/// The global degree of freedom of translation `d` (0–2) of node `n` in the system's uniform,
/// node-major layout ([`NODE_DOFS`] per node, checked when the system is built).
pub fn dof(n: usize, d: usize) -> usize {
    NODE_DOFS * n + d
}

/// The node and the direction (0–2) of a degree of freedom: the inverse of [`dof`].
pub fn node_direction(dof: usize) -> (usize, usize) {
    (dof / NODE_DOFS, dof % NODE_DOFS)
}

/// A spring or dashpot on degrees of freedom: the elongation is Σ c u(second) − Σ c u(first).
#[derive(Debug, Clone)]
pub struct Link {
    pub name: String,
    pub dashpot: bool,
    /// (degree of freedom, coefficient) with the first end's coefficients negated.
    pub terms: Vec<(usize, f64)>,
    pub value: f64,
    /// For one along the line of its ends: the two nodes and their rest distance.
    pub line: Option<(usize, usize, f64)>,
}

impl Link {
    fn stretch(&self, x: &[f64]) -> f64 {
        self.terms.iter().map(|(d, c)| c * x.get(*d).copied().unwrap_or(0.0)).sum()
    }

    /// Add its internal force (spring on `u`, dashpot on `v`; `xyz` the rest positions of the
    /// system's nodes, for a line that turns under nonlinear geometry).
    pub fn forces(&self, xyz: &[[f64; 3]], u: &[f64], v: &[f64], nonlinear: bool, stiffness: &mut [f64], viscous: &mut [f64]) {
        if nonlinear && let Some((a, b, rest)) = self.line {
            let at = |n: usize, x: &[f64]| -> [f64; 3] { [0, 1, 2].map(|k| x.get(dof(n, k)).copied().unwrap_or(0.0)) };
            let pos = |n: usize| {
                let (p, d) = (xyz.get(n).copied().unwrap_or_default(), at(n, u));
                [p[0] + d[0], p[1] + d[1], p[2] + d[2]]
            };
            let (pa, pb) = (pos(a), pos(b));
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let t = if l > 0.0 { d.map(|c| c / l) } else { [0.0; 3] };
            let (va, vb) = (at(a, v), at(b, v));
            let s = if self.dashpot { self.value * (0..3).map(|k| t[k] * (vb[k] - va[k])).sum::<f64>() } else { self.value * (l - rest) };
            let out = if self.dashpot { viscous } else { stiffness };
            for k in 0..3 {
                if let Some(x) = out.get_mut(dof(a, k)) {
                    *x -= s * t[k];
                }
                if let Some(x) = out.get_mut(dof(b, k)) {
                    *x += s * t[k];
                }
            }
            return;
        }
        let (x, out) = if self.dashpot { (v, viscous) } else { (u, stiffness) };
        let s = self.value * self.stretch(x);
        for (d, c) in &self.terms {
            if let Some(f) = out.get_mut(*d) {
                *f += s * c;
            }
        }
    }
}

/// Springs, dashpots and point masses of a model on the explicit system's nodes.
#[derive(Debug, Clone, Default)]
pub struct Discrete {
    pub links: Vec<Link>,
    /// Point mass per degree of freedom.
    pub mass: Vec<f64>,
}

impl Discrete {
    /// `node` maps a label to its node index.
    /// `node` maps a label to its node index, `xyz` gives the nodes' rest positions.
    pub fn new(resolved: &Resolved, dofs: usize, xyz: &[[f64; 3]], node: impl Fn(u32) -> Option<usize>) -> Result<Discrete> {
        let off = |what: &str, name: &str, label: u32| {
            Error::NotSupported(format!(
                "{what} `{name}` at node {label}: explicit dynamics takes discrete elements on mesh nodes (not reference points) so far"
            ))
        };
        let mut out = Discrete { links: Vec::new(), mass: vec![0.0; dofs] };
        for m in &resolved.masses {
            if m.inertia.iter().any(|c| *c != 0.0) {
                return Err(Error::NotSupported(format!("rotary inertia (point mass `{}`) in explicit dynamics: it has no rotations yet", m.name)));
            }
            let n = node(m.node).ok_or_else(|| off("point mass", &m.name, m.node))?;
            for d in 0..3 {
                if let Some(x) = out.mass.get_mut(dof(n, d)) {
                    *x += m.mass[d];
                }
            }
        }
        for s in &resolved.springs {
            let what = s.kind();
            if s.direction.is_rotational() {
                return Err(Error::NotSupported(format!("rotational {what}s (`{}`) in explicit dynamics: it has no rotations yet", s.name)));
            }
            let c = s.direction.coefficients();
            let mut terms = Vec::new();
            let mut ends = Vec::new();
            for (label, sign) in std::iter::once((s.first, -1.0)).chain(s.second.map(|b| (b, 1.0))) {
                let n = node(label).ok_or_else(|| off(what, &s.name, label))?;
                ends.push(n);
                terms.extend((0..3).filter(|d| c[*d] != 0.0).map(|d| (dof(n, d), sign * c[d])));
            }
            let line = match (s.direction, ends.as_slice()) {
                (ResolvedDirection::Line(_), [a, b]) => {
                    let (pa, pb) = (xyz.get(*a).copied().unwrap_or_default(), xyz.get(*b).copied().unwrap_or_default());
                    Some((*a, *b, ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2) + (pb[2] - pa[2]).powi(2)).sqrt()))
                }
                _ => None,
            };
            out.links.push(Link { name: s.name.clone(), dashpot: s.dashpot, terms, value: s.value, line });
        }
        Ok(out)
    }

    /// The smallest stable increment of the springs and dashpots on the lumped mass `mass` (per
    /// degree of freedom), and the name of a link at the degree of freedom that sets it. A link
    /// along its turning line (`nonlinear`) is bounded for every direction it may take.
    pub fn critical_increment(&self, mass: &[f64], nonlinear: bool) -> Option<(f64, String)> {
        let mut stiff = vec![0.0; mass.len()];
        let mut damp = vec![0.0; mass.len()];
        let mut owner: Vec<Option<usize>> = vec![None; mass.len()];
        for (k, l) in self.links.iter().enumerate() {
            let rows = if l.dashpot { &mut damp } else { &mut stiff };
            let turning: Vec<(usize, f64)>;
            let terms = match l.line {
                Some((a, b, _)) if nonlinear => {
                    turning = (0..3).map(|k| (dof(a, k), -1.0)).chain((0..3).map(|k| (dof(b, k), 1.0))).collect();
                    &turning
                }
                _ => &l.terms,
            };
            for (i, ci) in terms {
                let mi = mass.get(*i).copied().unwrap_or(0.0);
                let sum: f64 = terms.iter().map(|(j, cj)| (l.value * ci * cj).abs() / (mi * mass.get(*j).copied().unwrap_or(0.0)).sqrt()).sum();
                if let Some(r) = rows.get_mut(*i) {
                    *r += sum;
                }
                if let Some(o) = owner.get_mut(*i) {
                    o.get_or_insert(k);
                }
            }
        }
        let mut best: Option<(f64, usize)> = None;
        for (i, (w2, d)) in stiff.iter().zip(&damp).enumerate() {
            let dt = match (*w2 > 0.0, *d > 0.0) {
                (false, false) => continue,
                (false, true) => 2.0 / d,
                (true, _) => {
                    let w = w2.sqrt();
                    let xi = d / (2.0 * w);
                    (2.0 / w) * ((1.0 + xi * xi).sqrt() - xi)
                }
            };
            if best.is_none_or(|(b, _)| dt < b) {
                best = Some((dt, i));
            }
        }
        let (dt, i) = best?;
        let name = owner.get(i).copied().flatten().and_then(|k| self.links.get(k)).map(|l| l.name.clone()).unwrap_or_default();
        Some((dt, name))
    }
}
