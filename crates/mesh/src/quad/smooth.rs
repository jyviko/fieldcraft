//! Constrained Laplacian smoothing of a planar quadrilateral mesh.
//!
//! Each free node moves towards the average of its edge neighbours (Laplacian smoothing), and
//! the move is kept only when it does not lower the smallest scaled Jacobian of the node's
//! quadrilaterals and keeps them valid by the exact corner test ("smart" Laplacian: Freitag,
//! *On combining Laplacian and optimization-based mesh smoothing techniques*, AMD-Vol. 220,
//! ASME 1997; Canann, Tristano, Staten, *An approach to combined Laplacian and optimization-based
//! smoothing for triangular, quadrilateral, and quad-dominant meshes*, IMR 1998). Half and quarter
//! steps are tried when the full step is refused. Nodes of quadrilaterals that are already
//! invalid accept any step that raises their smallest corner area (see below).
//!
//! Laplacian smoothing cannot untangle a node whose average position is itself bad, and the
//! nodes next to a sharp prescribed boundary point often are. Every node whose quadrilaterals
//! remain inverted or poor (smallest scaled Jacobian below [`POOR`]) then gets optimization-based
//! smoothing (Freitag 1997) by a compass search (eight directions, step halved on failure,
//! bounded). An inverted patch is untangled first by maximizing its smallest corner area, which is
//! linear in the node's position, so its maximum lies in the feasible region when there is one
//! (Freitag and Plassmann, *Local optimization-based simplicial mesh untangling and
//! improvement*, IJNME 49:109–125, 2000); a valid patch then maximizes its smallest scaled
//! Jacobian without leaving the valid set.
//!
//! Gauss–Seidel order by node id; bounded sweeps; deterministic.

use super::free::{is_valid, min_scaled_jacobian};
use crate::predicates::{Point2, snap_to_domain};
use crate::{Error, Result};

/// Compressed adjacency: `items[start[v]..start[v + 1]]`.
struct Csr {
    start: Vec<usize>,
    items: Vec<u32>,
}

impl Csr {
    fn of(n: usize, pairs: &mut [(u32, u32)]) -> Csr {
        pairs.sort_unstable();
        let mut start = vec![0usize; n + 1];
        let mut items = Vec::with_capacity(pairs.len());
        let mut prev = None;
        for &(a, b) in pairs.iter() {
            if prev == Some((a, b)) {
                continue;
            }
            prev = Some((a, b));
            if let Some(s) = start.get_mut(a as usize + 1) {
                *s += 1;
            }
            items.push(b);
        }
        for i in 0..n {
            let before = start.get(i).copied().unwrap_or(0);
            if let Some(s) = start.get_mut(i + 1) {
                *s += before;
            }
        }
        Csr { start, items }
    }

    fn get(&self, v: usize) -> &[u32] {
        match (self.start.get(v), self.start.get(v + 1)) {
            (Some(&a), Some(&b)) => self.items.get(a..b).unwrap_or(&[]),
            _ => &[],
        }
    }
}

fn corners(nodes: &[[f64; 2]], q: &[u32; 4], v: u32, at: [f64; 2]) -> Result<[[f64; 2]; 4]> {
    let mut c = [[0.0; 2]; 4];
    for (k, &u) in q.iter().enumerate() {
        c[k] = if u == v { at } else { nodes.get(u as usize).copied().ok_or_else(|| Error::Failed("node index".into()))? };
    }
    Ok(c)
}

/// The state of `v`'s quadrilaterals with `v` at some position.
#[derive(Debug, Clone, Copy)]
struct Local {
    /// Smallest scaled Jacobian.
    sj: f64,
    /// Smallest unnormalized corner cross product among the corners that move with `v` (linear
    /// in `v`'s position: the untangling objective).
    area: f64,
    /// All valid by the exact test.
    valid: bool,
}

impl Local {
    /// Valid beats invalid; then a valid patch ranks by its worst scaled Jacobian and an invalid
    /// one by its worst corner area.
    fn key(&self) -> (bool, f64) {
        (self.valid, if self.valid { self.sj } else { self.area })
    }
    fn better(&self, than: &Local) -> bool {
        let (a, b) = (self.key(), than.key());
        (a.0 && !b.0) || (a.0 == b.0 && a.1 > b.1)
    }
    fn no_worse(&self, than: &Local) -> bool {
        !than.better(self)
    }
}

/// `v`'s quadrilaterals with `v` at `at`.
fn local(nodes: &[[f64; 2]], quads: &[[u32; 4]], incident: &[u32], v: u32, at: [f64; 2]) -> Result<Local> {
    let mut l = Local { sj: f64::INFINITY, area: f64::INFINITY, valid: true };
    for &e in incident {
        let quad = quads.get(e as usize).ok_or_else(|| Error::Failed("quad index".into()))?;
        let c = corners(nodes, quad, v, at)?;
        l.sj = l.sj.min(min_scaled_jacobian(c));
        if let Some(k) = quad.iter().position(|&u| u == v) {
            for i in [k + 3, k, k + 1] {
                let (a, b, d) = (c[(i + 3) % 4], c[i % 4], c[(i + 1) % 4]);
                let cross = (d[0] - b[0]) * (a[1] - b[1]) - (d[1] - b[1]) * (a[0] - b[0]);
                l.area = l.area.min(cross);
            }
        }
        if l.valid {
            let [a, b, c, d] = c.map(|x| Point2::new(x[0], x[1]));
            l.valid = is_valid([&a?, &b?, &c?, &d?]);
        }
    }
    Ok(l)
}

/// Nodes whose quadrilaterals score below this get optimization-based smoothing.
const POOR: f64 = 0.3;
/// Compass-search iterations per node.
const SEARCH_STEPS: usize = 48;

/// Compass search for the best position of `v` ([`Local::key`]: untangle first, then maximize
/// the smallest scaled Jacobian). Returns the new position when it improves.
fn optimize(nodes: &[[f64; 2]], quads: &[[u32; 4]], incident: &[u32], neighbours: &[u32], v: u32) -> Result<Option<[f64; 2]>> {
    let old = nodes.get(v as usize).copied().ok_or_else(|| Error::Failed("node index".into()))?;
    let mut reach = 0.0_f64;
    for &u in neighbours {
        let p = nodes.get(u as usize).copied().ok_or_else(|| Error::Failed("node index".into()))?;
        reach = reach.max(((p[0] - old[0]).powi(2) + (p[1] - old[1]).powi(2)).sqrt());
    }
    let start = local(nodes, quads, incident, v, old)?;
    if start.valid && start.sj >= POOR {
        return Ok(None);
    }
    let (mut at, mut best) = (old, start);
    let mut step = 0.25 * reach;
    let floor = 1e-4 * step;
    const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
    const DIRS: [[f64; 2]; 8] = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [D, D], [-D, D], [-D, -D], [D, -D]];
    for _ in 0..SEARCH_STEPS {
        if !(step > floor) {
            break;
        }
        let mut moved = false;
        for d in DIRS {
            let to = [snap_to_domain(at[0] + step * d[0]), snap_to_domain(at[1] + step * d[1])];
            let l = local(nodes, quads, incident, v, to)?;
            if l.better(&best) {
                (at, best) = (to, l);
                moved = true;
                break;
            }
        }
        if !moved {
            step *= 0.5;
        }
    }
    Ok((at != old && best.better(&start)).then_some(at))
}

/// Smooth the nodes from `first_free` on for at most `sweeps` sweeps. Returns the sweeps run and
/// whether the largest move of the last sweep fell below `1e-4` of the mean edge length.
pub(crate) fn smooth(nodes: &mut [[f64; 2]], quads: &[[u32; 4]], first_free: usize, sweeps: usize) -> Result<(usize, bool)> {
    let n = nodes.len();
    let mut edges = Vec::with_capacity(8 * quads.len());
    let mut inc = Vec::with_capacity(4 * quads.len());
    let mut total = 0.0;
    for (e, q) in quads.iter().enumerate() {
        for k in 0..4 {
            let (a, b) = (q[k], q[(k + 1) % 4]);
            edges.push((a, b));
            edges.push((b, a));
            inc.push((a, e as u32));
            if let (Some(pa), Some(pb)) = (nodes.get(a as usize), nodes.get(b as usize)) {
                total += ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt();
            }
        }
    }
    if quads.is_empty() {
        return Ok((0, true));
    }
    let tolerance = 1e-4 * total / (4 * quads.len()) as f64;
    let neighbours = Csr::of(n, &mut edges);
    let incident = Csr::of(n, &mut inc);
    for sweep in 0..sweeps {
        let mut largest = 0.0_f64;
        for v in first_free..n {
            let nb = neighbours.get(v);
            if nb.is_empty() {
                continue;
            }
            let old = nodes.get(v).copied().ok_or_else(|| Error::Failed("node index".into()))?;
            let mut avg = [0.0, 0.0];
            for &u in nb {
                let p = nodes.get(u as usize).copied().ok_or_else(|| Error::Failed("node index".into()))?;
                avg = [avg[0] + p[0], avg[1] + p[1]];
            }
            let avg = [avg[0] / nb.len() as f64, avg[1] / nb.len() as f64];
            let quads_of_v = incident.get(v);
            let before = local(nodes, quads, quads_of_v, v as u32, old)?;
            for step in [1.0, 0.5, 0.25] {
                let at = [snap_to_domain(old[0] + step * (avg[0] - old[0])), snap_to_domain(old[1] + step * (avg[1] - old[1]))];
                if at == old {
                    break;
                }
                if local(nodes, quads, quads_of_v, v as u32, at)?.no_worse(&before) {
                    largest = largest.max(((at[0] - old[0]).powi(2) + (at[1] - old[1]).powi(2)).sqrt());
                    if let Some(x) = nodes.get_mut(v) {
                        *x = at;
                    }
                    break;
                }
            }
        }
        if largest <= tolerance {
            let polished = polish(nodes, quads, first_free, &neighbours, &incident, sweeps)?;
            return Ok((sweep + 1, polished));
        }
    }
    polish(nodes, quads, first_free, &neighbours, &incident, sweeps)?;
    Ok((sweeps, false))
}

/// Optimization-based smoothing of the poor and inverted nodes, at most `sweeps` sweeps.
/// Returns whether a sweep ended with nothing left to improve.
fn polish(nodes: &mut [[f64; 2]], quads: &[[u32; 4]], first_free: usize, neighbours: &Csr, incident: &Csr, sweeps: usize) -> Result<bool> {
    for _ in 0..sweeps {
        let mut changed = false;
        for v in first_free..nodes.len() {
            if let Some(at) = optimize(nodes, quads, incident.get(v), neighbours.get(v), v as u32)? {
                if let Some(x) = nodes.get_mut(v) {
                    *x = at;
                }
                changed = true;
            }
        }
        if !changed {
            return Ok(true);
        }
    }
    Ok(false)
}
