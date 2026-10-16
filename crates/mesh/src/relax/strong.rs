//! Strong relaxation: an optimisation of the node positions of a whole region toward elements of
//! scaled Jacobian 1 (`mesh.node.relax` with `"mode": "strong"`).
//!
//! The unknowns are the positions of the region's corner nodes: interior nodes in space, nodes on
//! a geometry face in the face's parameters ([`Constraints::slide_face`]); nodes on geometry edges
//! and vertices are held, so feature lines stay as they are. The objective is
//!
//! ```text
//! F = Σₑ φ(qₑ) + ½ ln²(Vₑ/V₀ₑ)   for a valid element (q its scaled Jacobian, V its size),
//! φ(q) = (1/q − 1)²               phase 1 (smooth): every element toward 1, the worst most,
//! φ(q) = q⁻¹⁶ − 1                 phase 2 (max–min): a smooth stand-in for the smallest q,
//! B (2 − q)                        for an invalid element (B large),
//! ```
//!
//! growing without bound as an element degenerates (a barrier: no step that lowers `F` inverts an
//! element), while the invalid branch falls as an inverted element untangles and lies far above
//! any valid configuration. The size term is there because the scaled Jacobian does not see an
//! element flattening along one direction: without it the optimiser squeezed elements against
//! the boundary to square up their neighbours (the Pi 5 base's largest edge ratio went from 5 to
//! 34). For the same reason an element longer than the region's largest edge ratio before costs
//! as much as an invalid one.
//!
//! `F` is minimised by block coordinate descent (nonlinear Gauss–Seidel): each sweep visits the
//! movable nodes in label order and lowers the part of `F` that depends on the node, first by
//! trying the centroid of its edge neighbours (Laplacian), then by a gradient step (central
//! differences, backtracking line search, the step length adapted per node). A move is kept when
//! it lowers the local objective, so individual elements may get worse on the way. L. Freitag,
//! P. Plassmann, *Local optimization-based simplicial mesh untangling and improvement*, IJNME 49,
//! 2000; P. Knupp, *Hexahedral and tetrahedral mesh untangling*, Eng. Comput. 17, 2001 (the
//! barrier objective on the Jacobian); J. Nocedal, S. Wright, *Numerical Optimization*, ch. 3
//! (backtracking).
//!
//! Phase 1 has up to half the sweeps, phase 2 (from phase 1's best state) the rest; a phase stops
//! when a sweep lowers its objective by less than a relative 1e-5, and after the first sweep only
//! nodes whose neighbourhood moved are visited. The sweeps keep the best state they pass through (highest smallest quality, then highest
//! mean). The result is that state if it has no invalid element and its smallest quality is not
//! below the starting one; otherwise nothing moves. Deterministic: no randomness, fixed order.

use std::collections::{BTreeMap, BTreeSet};

use fieldcraft_model::Model;

use super::{Constraints, Elem, Freedom, P, RegionQuality, State, add, dist, dot, measure, region_quality, scale, sub};
use crate::{Error, Result};

/// Most sweeps of a strong relaxation.
pub const MAX_SWEEPS: usize = 2000;
/// Weight of an invalid element in the objective.
const INVALID: f64 = 1e8;
/// A phase stops when a sweep lowers its objective by less than this fraction.
const CONVERGED: f64 = 1e-5;

/// One sweep's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sweep {
    pub phase: Phase,
    /// The phase's objective.
    pub objective: f64,
    pub min_quality: f64,
    pub mean_quality: f64,
    pub invalid: usize,
}

/// What [`optimize`] did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Optimized {
    /// The new position of every node that moved.
    pub positions: BTreeMap<u32, P>,
    /// Corner nodes that moved.
    pub moved: usize,
    pub max_displacement: f64,
    pub before: RegionQuality,
    pub after: RegionQuality,
    pub mean_before: f64,
    pub mean_after: f64,
    /// Before the first sweep, then after each.
    pub history: Vec<Sweep>,
    /// The sweep whose state was kept (0: the start, nothing moved).
    pub kept: usize,
}

/// Weight of the size term.
const SIZE_WEIGHT: f64 = 0.5;

/// Exponent of the max–min phase.
const MIN_MAX_POWER: i32 = 16;

/// What a phase minimises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// `(1/q − 1)²`: every element toward 1, the worst weighted most.
    #[default]
    Smooth,
    /// `q⁻¹⁶`: a smooth stand-in for the smallest quality, which it lifts.
    MinMax,
}

/// The objective's term for one element of quality `q` whose size went from `v0` to `v`.
fn phi(phase: Phase, q: f64, valid: bool, v: f64, v0: f64) -> f64 {
    if valid && q > 0.0 {
        let g = if v > 0.0 && v0 > 0.0 { (v / v0).ln() } else { 0.0 };
        let shape = match phase {
            Phase::Smooth => (1.0 / q - 1.0).powi(2),
            Phase::MinMax => q.powi(-MIN_MAX_POWER) - 1.0,
        };
        shape + SIZE_WEIGHT * g * g
    } else {
        INVALID * (2.0 - q.min(1.0))
    }
}

/// A linear hexahedron's scaled Jacobian at its corners and centre (the samples
/// [`crate::quality::element_quality`] takes), straight from the trilinear map: the sweeps'
/// inner loop, where the certified check would dominate. A state is certified before it is kept.
fn hex8_scaled_jacobian(x: &[P]) -> (f64, f64) {
    let corners = &crate::quality::jacobian::HEX_CORNERS;
    let at = |k: usize| x.get(k).copied().unwrap_or([0.0; 3]);
    let (mut worst, mut centre) = (f64::INFINITY, 0.0);
    for u in corners.iter().map(|c| c.map(|v| v as f64)).chain(std::iter::once([0.5; 3])) {
        let mut col = [[0.0; 3]; 3];
        for (k, bits) in corners.iter().enumerate() {
            for (d, c) in col.iter_mut().enumerate() {
                let w: f64 = (0..3)
                    .map(|e| {
                        let b = bits[e] as f64;
                        if e == d { 2.0 * b - 1.0 } else { b * u[e] + (1.0 - b) * (1.0 - u[e]) }
                    })
                    .product();
                *c = add(*c, scale(at(k), w));
            }
        }
        let l = dot(col[0], col[0]).sqrt() * dot(col[1], col[1]).sqrt() * dot(col[2], col[2]).sqrt();
        let det = dot(col[0], super::cross(col[1], col[2]));
        let q = if l > 0.0 && l.is_finite() { det / l } else { -1.0 };
        worst = worst.min(q);
        centre = det;
    }
    (worst, centre)
}

/// An element's size for the size term: a linear hexahedron's Jacobian at its centre (its
/// volume, near enough), else the size [`crate::quality::element_quality`] reports.
pub(super) fn size(kind: fieldcraft_model::ElementKind, x: &[P]) -> f64 {
    if kind == fieldcraft_model::ElementKind::Hex8 {
        hex8_scaled_jacobian(x).1
    } else {
        crate::quality::element_quality(kind, x).map_or(0.0, |q| q.size)
    }
}

/// An element's objective term, quality and validity; `certified` takes the certified check
/// for every kind.
fn term_of(phase: Phase, e: &Elem, s: &State<'_>, certified: bool) -> (f64, f64, bool) {
    let x = s.coords(e);
    let hex = e.kind == fieldcraft_model::ElementKind::Hex8;
    let (q, valid, v) = if !certified && hex {
        let (q, v) = hex8_scaled_jacobian(&x);
        (q, q > 0.0, v)
    } else {
        let (q, valid) = measure(e, &x);
        (q, valid, size(e.kind, &x))
    };
    // An element stretched past the cap is as bad as an invalid one.
    let ratio = super::edge_ratio(e.kind, &x);
    let stretch = if ratio > s.ratio_cap { INVALID * ratio / s.ratio_cap } else { 0.0 };
    (phi(phase, q, valid, v, e.size0) + stretch, q, valid)
}

impl State<'_> {
    /// The objective over the elements at `n`.
    fn local_objective(&self, phase: Phase, n: u32) -> f64 {
        let mut f = 0.0;
        for &i in self.around.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(e) = self.elems.get(i) {
                f += term_of(phase, e, self, false).0;
            }
        }
        f
    }

    /// `n` moved by `d` under `freedom`, kept when the local objective drops below `f0`; the new
    /// local objective when kept.
    fn try_lower(&mut self, phase: Phase, n: u32, freedom: Freedom, d: P, f0: f64) -> Option<f64> {
        let p = self.at(n);
        let q = self.moved(freedom, p, d)?;
        if dist(p, q) == 0.0 {
            return None;
        }
        let undo = self.place(n, q);
        let f = self.local_objective(phase, n);
        if f < f0 {
            Some(f)
        } else {
            self.restore(undo);
            None
        }
    }

    /// One optimisation of node `n`; `step` is its adaptive step (relative to the local edge
    /// length). Whether it moved.
    fn optimize_node(&mut self, phase: Phase, n: u32, step: &mut f64) -> bool {
        let freedom = self.freedom(n);
        if freedom == Freedom::Fixed {
            return false;
        }
        let mut f0 = self.local_objective(phase, n);
        if f0 == 0.0 {
            return false;
        }
        let mut moved = false;
        // Laplacian: the centroid of the edge neighbours, kept when it lowers the objective.
        let nb: Vec<u32> = self.neighbours.get(&n).map(|s| s.iter().copied().collect()).unwrap_or_default();
        if !nb.is_empty() {
            let c = scale(nb.iter().fold([0.0; 3], |s, &m| add(s, self.at(m))), 1.0 / nb.len() as f64);
            let d = sub(c, self.at(n));
            for k in [1.0, 0.5, 0.25] {
                if let Some(f) = self.try_lower(phase, n, freedom, scale(d, k), f0) {
                    f0 = f;
                    moved = true;
                    break;
                }
            }
        }
        // Gradient step (central differences in space; on a face the slide maps the difference
        // onto the surface, so the gradient is the surface gradient).
        let h = self.local_length(n);
        if !(h > 0.0) {
            return moved;
        }
        let here = self.at(n);
        let eps = 1e-5 * h;
        let mut g = [0.0; 3];
        for k in 0..3 {
            let mut f = [0.0; 2];
            for (j, sign) in [1.0, -1.0].into_iter().enumerate() {
                let mut d = [0.0; 3];
                d[k] = sign * eps;
                f[j] = match self.moved(freedom, here, d) {
                    Some(q) => {
                        let undo = self.place(n, q);
                        let v = self.local_objective(phase, n);
                        self.restore(undo);
                        v
                    }
                    None => f0,
                };
            }
            g[k] = (f[0] - f[1]) / (2.0 * eps);
        }
        let gn = dot(g, g).sqrt();
        if !(gn > 0.0 && gn.is_finite()) {
            return moved;
        }
        let dir = scale(g, -1.0 / gn);
        let mut t = *step;
        for attempt in 0..8 {
            if let Some(_f) = self.try_lower(phase, n, freedom, scale(dir, t * h), f0) {
                // Grow after a first-try success, keep the shortened step otherwise.
                *step = if attempt == 0 { (t * 2.0).min(0.5) } else { t };
                return true;
            }
            t *= 0.5;
        }
        *step = (t * 2.0).max(1e-6);
        moved
    }

    fn sweep_numbers(&self, phase: Phase) -> Sweep {
        let mut s = Sweep { phase, min_quality: f64::INFINITY, ..Sweep::default() };
        for e in &self.elems {
            let (f, q, valid) = term_of(phase, e, self, true);
            s.objective += f;
            s.min_quality = s.min_quality.min(q);
            s.mean_quality += q;
            s.invalid += usize::from(!valid);
        }
        if self.elems.is_empty() {
            s.min_quality = 0.0;
        } else {
            s.mean_quality /= self.elems.len() as f64;
        }
        s
    }
}

/// Whether `a` is a better state than `b`: fewer invalid elements, then a higher smallest
/// quality, then a higher mean.
fn better(a: &Sweep, b: &Sweep) -> bool {
    a.invalid < b.invalid
        || (a.invalid == b.invalid && (a.min_quality > b.min_quality || (a.min_quality == b.min_quality && a.mean_quality > b.mean_quality)))
}

/// Optimise the corner nodes `nodes` (and the mid-side nodes following them) for up to `sweeps`
/// sweeps, faces' nodes sliding on them and edges' and vertices' nodes held, as `geometry` says.
/// The model is not changed: the result has the new positions.
pub fn optimize(model: &Model, nodes: &BTreeSet<u32>, sweeps: usize, geometry: &dyn Constraints) -> Result<Optimized> {
    if sweeps == 0 || sweeps > MAX_SWEEPS {
        return Err(Error::Invalid(format!("sweeps must be 1 to {MAX_SWEEPS}, got {sweeps}")));
    }
    if nodes.len() > super::MAX_NODES {
        return Err(Error::Limit(format!("more than {} nodes to relax", super::MAX_NODES)));
    }
    let mut state = State::new(model, nodes, geometry, true)?;
    let original = state.pos.clone();
    let before = region_quality(&state);
    // No element may end up more stretched than the most stretched one was.
    state.ratio_cap = before.max_edge_ratio.max(1.0);
    let start = state.sweep_numbers(Phase::Smooth);
    let mut history = vec![start];
    let (mut best, mut best_pos, mut kept) = (start, None::<BTreeMap<u32, P>>, 0);
    let movers: Vec<u32> = nodes.iter().copied().filter(|n| state.around.contains_key(n) && state.freedom(*n) != Freedom::Fixed).collect();
    let mut steps: BTreeMap<u32, f64> = movers.iter().map(|&n| (n, 0.1)).collect();
    // Phase 1 has up to half the sweeps (the rest when it stops early); phase 2 the rest. Each
    // sweep visits the nodes whose neighbourhood changed in the previous one.
    let mut k = 0;
    for (phase, budget) in [(Phase::Smooth, sweeps.div_ceil(2)), (Phase::MinMax, sweeps)] {
        let mut active: BTreeSet<u32> = movers.iter().copied().collect();
        let mut last = state.sweep_numbers(phase).objective;
        while k < budget && !active.is_empty() {
            k += 1;
            let mut next = BTreeSet::new();
            for &n in &movers {
                if !active.contains(&n) {
                    continue;
                }
                let mut step = steps.get(&n).copied().unwrap_or(0.1);
                if state.optimize_node(phase, n, &mut step) {
                    next.insert(n);
                    next.extend(state.neighbours.get(&n).into_iter().flatten().copied());
                }
                steps.insert(n, step);
            }
            active = next;
            let now = state.sweep_numbers(phase);
            history.push(now);
            if better(&now, &best) {
                best = now;
                best_pos = Some(state.pos.clone());
                kept = k;
            }
            // Converged: the objective no longer drops.
            if last - now.objective <= CONVERGED * last.max(f64::MIN_POSITIVE) {
                break;
            }
            last = now.objective;
        }
        // Phase 2 starts from the best state.
        if let Some(p) = &best_pos {
            state.pos = p.clone();
        }
    }
    let accept = best.invalid == 0 && best.min_quality >= start.min_quality;
    match best_pos {
        Some(p) if accept => state.pos = p,
        _ => {
            state.pos = original.clone();
            kept = 0;
        }
    }
    let after = region_quality(&state);
    let end = state.sweep_numbers(Phase::Smooth);
    let mut out = Optimized { before, after, mean_before: start.mean_quality, mean_after: end.mean_quality, history, kept, ..Default::default() };
    for (n, p) in &state.pos {
        let Some(o) = original.get(n) else { continue };
        let d = dist(*o, *p);
        if d > 0.0 {
            out.max_displacement = out.max_displacement.max(d);
            out.moved += usize::from(nodes.contains(n));
            out.positions.insert(*n, *p);
        }
    }
    Ok(out)
}
