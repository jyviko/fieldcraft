//! Planar face meshing for the sweepers: the scheme a region gets, the interval constraints the
//! scheme needs, and the quadrilateral mesh on the assigned boundary nodes.
//!
//! - **Mapped**: one loop with four corners (vertex classification by interior angle, as in
//!   submapping: Whiteley, White, Benzley, Blacker, *Two and three-quarter dimensional meshing
//!   facilitators*, Engineering with Computers 12:144–154, 1996): opposite sides equal,
//!   transfinite interpolation ([`crate::quad::mapped_quad_mesh`]).
//! - **Annulus**: two smooth loops (no corners), one inside the other: the two loops get equal
//!   counts and the ring between them is a periodic structured grid, each radial line joining a
//!   node of the outer loop to its partner on the inner loop (the rotation that pairs them most
//!   closely) — the O-grid of a tube's cross-section.
//! - **Free**: anything else: even loops for the free all-quad mesher
//!   ([`crate::quad::free_quad_mesh`]).
//!
//! A structured scheme whose grid would fold falls back to the free mesher (its constraints imply
//! even loops).

use std::f64::consts::PI;

use super::interval::Intervals;
use crate::quad::{QuadMesh, QuadOptions, QuadStats, free_quad_mesh, mapped_quad_mesh};
use crate::quality::{Validity, planar_quad_validity};
use crate::size::SizeField;
use crate::{Error, Result};

type Q = [f64; 2];

/// The most nodes the annulus pairing compares (its cost is quadratic in the loop length).
const MAX_ANNULUS_NODES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    /// Corner positions (curve uses) in the single loop.
    Mapped([usize; 4]),
    Annulus,
    Free,
}

/// The interior angle at every curve end of a loop, given each curve use's oriented polyline
/// (with the loop counter-clockwise when `ccw`).
fn angles(lp: &[Vec<Q>], ccw: bool) -> Option<Vec<f64>> {
    let n = lp.len();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let prev = lp.get((k + n - 1) % n)?;
        let next = lp.get(k)?;
        let (a, o, b) = (*prev.get(prev.len().checked_sub(2)?)?, *next.first()?, *next.get(1)?);
        let (u, v) = ([o[0] - a[0], o[1] - a[1]], [b[0] - o[0], b[1] - o[1]]);
        let turn = (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]);
        out.push(PI - if ccw { turn } else { -turn });
    }
    Some(out)
}

fn signed_area(pts: &[Q]) -> f64 {
    let n = pts.len();
    0.5 * (0..n).map(|k| pts[k][0] * pts[(k + 1) % n][1] - pts[(k + 1) % n][0] * pts[k][1]).sum::<f64>()
}

/// Choose the scheme of a region from its loops: per loop, the oriented polyline of each curve
/// use (loop 0 outer).
pub(crate) fn classify(loops: &[Vec<Vec<Q>>]) -> Scheme {
    let ccw = |lp: &Vec<Vec<Q>>| {
        let pts: Vec<Q> = lp.iter().flat_map(|c| c.iter().take(c.len().saturating_sub(1)).copied()).collect();
        signed_area(&pts) > 0.0
    };
    match loops {
        [lp] => {
            let Some(a) = angles(lp, ccw(lp)) else { return Scheme::Free };
            if a.iter().any(|&x| x >= 1.25 * PI) {
                return Scheme::Free;
            }
            let corners: Vec<usize> = (0..a.len()).filter(|&k| a.get(k).is_some_and(|&x| x <= 0.75 * PI)).collect();
            corners.try_into().map(Scheme::Mapped).unwrap_or(Scheme::Free)
        }
        [outer, inner] => {
            let smooth = |lp: &Vec<Vec<Q>>| angles(lp, ccw(lp)).is_some_and(|a| a.iter().all(|&x| x > 0.75 * PI && x < 1.25 * PI));
            if smooth(outer) && smooth(inner) { Scheme::Annulus } else { Scheme::Free }
        }
        _ => Scheme::Free,
    }
}

/// Add the scheme's constraints on the loops' curve variables.
pub(crate) fn constrain(ia: &mut Intervals, loops: &[Vec<usize>], scheme: Scheme) {
    match scheme {
        Scheme::Mapped(c) => {
            let Some(lp) = loops.first() else { return };
            let side = |a: usize, b: usize| (a..b).filter_map(|k| lp.get(k).copied()).collect::<Vec<_>>();
            ia.equal.push((side(c[0], c[1]), side(c[2], c[3])));
            ia.equal.push((side(c[1], c[2]), side(c[3], lp.len()).into_iter().chain(side(0, c[0])).collect()));
        }
        Scheme::Annulus => {
            if let (Some(o), Some(i)) = (loops.first(), loops.get(1)) {
                ia.equal.push((o.clone(), i.clone()));
                ia.even.push(o.clone());
                ia.at_least.push((o.clone(), 6));
            }
        }
        Scheme::Free => {
            let min = if loops.len() > 1 { 6 } else { 4 };
            for lp in loops {
                ia.even.push(lp.clone());
                ia.at_least.push((lp.clone(), min));
            }
        }
    }
}

/// Mesh a region on its boundary nodes: `loops[k]` are loop `k`'s nodes in order, `starts[k]` the
/// positions where its curve uses start. Returns the mesh (boundary nodes first, in the order of
/// the loops) and the scheme used.
pub(crate) fn mesh(loops: &[Vec<Q>], starts: &[Vec<usize>], scheme: Scheme, size: &dyn SizeField, opts: &QuadOptions) -> Result<(QuadMesh, Scheme)> {
    let structured = match scheme {
        Scheme::Mapped(c) => {
            let s0 = starts.first();
            let pos = c.map(|k| s0.and_then(|s| s.get(k)).copied().unwrap_or(0));
            loops.first().and_then(|b| mapped_quad_mesh(b, pos, opts).ok()).map(|m| m.mesh)
        }
        Scheme::Annulus => annulus(loops, size, opts)?,
        Scheme::Free => None,
    };
    match structured {
        Some(m) => Ok((m, scheme)),
        None => Ok((free_quad_mesh(loops, size, opts)?, Scheme::Free)),
    }
}

/// The O-grid between two loops of equal length, or `None` when it would fold.
fn annulus(loops: &[Vec<Q>], size: &dyn SizeField, opts: &QuadOptions) -> Result<Option<QuadMesh>> {
    let [outer, inner] = loops else { return Ok(None) };
    let n = outer.len();
    if n != inner.len() || !(3..=MAX_ANNULUS_NODES).contains(&n) {
        return Ok(None);
    }
    // Index maps that make both loops counter-clockwise.
    let ccw_order = |pts: &[Q]| -> Vec<usize> { if signed_area(pts) > 0.0 { (0..pts.len()).collect() } else { (0..pts.len()).rev().collect() } };
    let (oi, ii) = (ccw_order(outer), ccw_order(inner));
    let o = |k: usize| oi.get(k % n).and_then(|&i| outer.get(i)).copied().unwrap_or([0.0; 2]);
    let inn = |k: usize| ii.get(k % n).and_then(|&i| inner.get(i)).copied().unwrap_or([0.0; 2]);
    let d2 = |a: Q, b: Q| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let shift = (0..n)
        .min_by(|&s, &t| {
            let cost = |s: usize| (0..n).map(|k| d2(o(k), inn(k + s))).sum::<f64>();
            cost(s).total_cmp(&cost(t)).then(s.cmp(&t))
        })
        .unwrap_or(0);
    let mean_gap = (0..n).map(|k| d2(o(k), inn(k + shift)).sqrt()).sum::<f64>() / n as f64;
    let mid = |k: usize| {
        let (a, b) = (o(k), inn(k + shift));
        [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, 0.0]
    };
    let h = crate::size::checked_size(size, mid(0))?;
    let m = (mean_gap / h).round().max(1.0) as usize;
    let quads_count = n.saturating_mul(m);
    if quads_count > opts.max_elements {
        return Err(Error::Limit(format!("the mesh would have {quads_count} quadrilaterals; the limit is {}", opts.max_elements)));
    }
    // Node ids: outer loop 0..n, inner loop n..2n (input order), then the rings between.
    let id_outer = |k: usize| oi.get(k % n).copied().unwrap_or(0) as u32;
    let id_inner = |k: usize| (n + ii.get(k % n).copied().unwrap_or(0)) as u32;
    let mut nodes: Vec<Q> = outer.iter().chain(inner.iter()).copied().collect();
    let mut ring_ids: Vec<Vec<u32>> = vec![(0..n).map(id_outer).collect()];
    for r in 1..m {
        let t = r as f64 / m as f64;
        let mut ids = Vec::with_capacity(n);
        for k in 0..n {
            let (a, b) = (o(k), inn(k + shift));
            nodes.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
            ids.push(nodes.len() as u32 - 1);
        }
        ring_ids.push(ids);
    }
    ring_ids.push((0..n).map(|k| id_inner(k + shift)).collect());
    let at = |i: u32| nodes.get(i as usize).copied().unwrap_or([0.0; 2]);
    let mut quads = Vec::with_capacity(quads_count);
    for w in ring_ids.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        for k in 0..n {
            let (Some(&p0), Some(&p1), Some(&q1), Some(&q0)) = (a.get(k), a.get((k + 1) % n), b.get((k + 1) % n), b.get(k)) else {
                return Ok(None);
            };
            // Outer ring counter-clockwise, inner ring inside it: (p0, p1, q1, q0) turns left.
            let q = [p0, p1, q1, q0];
            if planar_quad_validity(&q.map(at))? != Validity::Valid {
                return Ok(None);
            }
            quads.push(q);
        }
    }
    let boundary_nodes = 2 * n;
    Ok(Some(QuadMesh { nodes, quads, boundary_nodes, stats: QuadStats::default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::UniformSize;

    fn circle(r: f64, n: usize, ccw: bool) -> Vec<Q> {
        (0..n)
            .map(|k| {
                let t = 2.0 * PI * k as f64 / n as f64 * if ccw { 1.0 } else { -1.0 };
                [r * t.cos(), r * t.sin()]
            })
            .collect()
    }

    #[test]
    fn rings_get_an_o_grid() {
        let (outer, inner) = (circle(5.0, 16, true), circle(4.0, 16, false));
        let polyline = |pts: &Vec<Q>| {
            let mut p = pts.clone();
            p.push(pts[0]);
            vec![p]
        };
        assert_eq!(classify(&[polyline(&outer), polyline(&inner)]), Scheme::Annulus);
        let (m, s) = mesh(&[outer.clone(), inner.clone()], &[vec![0], vec![0]], Scheme::Annulus, &UniformSize(0.5), &QuadOptions::default()).unwrap();
        assert_eq!(s, Scheme::Annulus);
        assert_eq!(m.quads.len(), 16 * 2);
        assert_eq!(m.boundary_nodes, 32);
        assert_eq!(&m.nodes[..16], &outer[..]);
        // Unequal loops fall back to the free mesher.
        let (_, s) =
            mesh(&[circle(5.0, 16, true), circle(2.0, 8, true)], &[vec![0], vec![0]], Scheme::Annulus, &UniformSize(1.0), &QuadOptions::default())
                .unwrap();
        assert_eq!(s, Scheme::Free);
    }

    #[test]
    fn squares_are_mapped() {
        let sq: Vec<Vec<Q>> =
            vec![vec![[0.0, 0.0], [1.0, 0.0]], vec![[1.0, 0.0], [1.0, 1.0]], vec![[1.0, 1.0], [0.0, 1.0]], vec![[0.0, 1.0], [0.0, 0.0]]];
        assert_eq!(classify(std::slice::from_ref(&sq)), Scheme::Mapped([0, 1, 2, 3]));
        let l: Vec<Vec<Q>> = vec![
            vec![[0.0, 0.0], [2.0, 0.0]],
            vec![[2.0, 0.0], [2.0, 1.0]],
            vec![[2.0, 1.0], [1.0, 1.0]],
            vec![[1.0, 1.0], [1.0, 2.0]],
            vec![[1.0, 2.0], [0.0, 2.0]],
            vec![[0.0, 2.0], [0.0, 0.0]],
        ];
        assert_eq!(classify(&[l]), Scheme::Free);
    }
}
