//! Where projected nodes go: onto a target (one B-rep face or a union of faces), onto the curve
//! where two targets meet, or onto the point where three meet.
//!
//! A curve or corner node converges by tangent planes: project the node onto each target,
//! intersect the targets' tangent planes there, move to the point of that intersection nearest
//! the node, repeat. For two transversal surfaces this is Newton's method on the curve; it needs
//! no B-rep edge between the faces, so it tolerates small gaps and overlaps in the CAD (the nodes
//! end on the curve where the two surfaces, extended, meet). Faces meeting at a shallow angle
//! fall back to alternating projections.

use crate::tet::geometry::Tessellation;
use crate::vec3::{P, add, cross, dist, dot, norm, scale, sub};

/// Rounds of the curve and corner iterations.
const ROUNDS: usize = 60;

/// Below this `|n_a × n_b|` two targets are treated as tangent (sin 3°).
const TANGENT_SIN: f64 = 0.052;

/// What the projection needs of the geometry.
pub trait Geometry {
    /// The closest point on B-rep face `face`.
    fn project(&self, face: u32, p: P) -> Option<P>;
    /// The unit normal of B-rep face `face` near `p`.
    fn normal(&self, face: u32, p: P) -> Option<P>;
}

impl Geometry for Tessellation {
    fn project(&self, face: u32, p: P) -> Option<P> {
        self.project_trimmed(face, p)
    }
    fn normal(&self, face: u32, p: P) -> Option<P> {
        self.face_normal(face, p)
    }
}

/// Where a node may go: targets by index into the projection's target list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Constraint {
    Free,
    Surface(usize),
    Curve(usize, usize),
    Corner(usize, usize, usize),
}

impl Constraint {
    /// The constraint of a node on the targets `on` (distinct, ascending).
    pub(crate) fn of(on: &[usize]) -> Constraint {
        match *on {
            [] => Constraint::Free,
            [a] => Constraint::Surface(a),
            [a, b] => Constraint::Curve(a, b),
            [a, b, c, ..] => Constraint::Corner(a, b, c),
        }
    }
}

/// The closest point to `p` on any face of `faces`, with its normal and face.
pub(crate) fn on_target(g: &dyn Geometry, faces: &[u32], p: P) -> Option<(P, P, u32)> {
    let mut best: Option<(f64, P, u32)> = None;
    for &f in faces {
        if let Some(q) = g.project(f, p) {
            let d = dist(p, q);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, q, f));
            }
        }
    }
    let (_, q, f) = best?;
    let n = g.normal(f, q)?;
    Some((q, n, f))
}

/// Place `p` by its constraint over `targets` (face lists); `tol` is the convergence distance.
/// A point the geometry cannot place stays where it is.
pub(crate) fn place(g: &dyn Geometry, targets: &[Vec<u32>], c: Constraint, p: P, tol: f64) -> P {
    let set = |i: usize| targets.get(i).map_or(&[][..], Vec::as_slice);
    match c {
        Constraint::Free => p,
        Constraint::Surface(a) => on_target(g, set(a), p).map_or(p, |h| h.0),
        Constraint::Curve(a, b) => planes(g, &[set(a), set(b)], p, tol),
        Constraint::Corner(a, b, c) => planes(g, &[set(a), set(b), set(c)], p, tol),
    }
}

/// The tangent-plane iteration onto the intersection of two or three targets.
fn planes(g: &dyn Geometry, sets: &[&[u32]], p: P, tol: f64) -> P {
    let mut x = p;
    for _ in 0..ROUNDS {
        let mut hits = Vec::with_capacity(sets.len());
        for s in sets {
            match on_target(g, s, x) {
                Some(h) => hits.push(h),
                None => return x,
            }
        }
        let next = match hits.as_slice() {
            [(qa, na, _), (qb, nb, _)] => {
                if norm(cross(*na, *nb)) < TANGENT_SIN {
                    alternate(g, sets, x)
                } else {
                    // The point of both planes nearest x: x − λa·na − λb·nb.
                    let m = dot(*na, *nb);
                    let (ra, rb) = (dot(*na, sub(x, *qa)), dot(*nb, sub(x, *qb)));
                    let den = 1.0 - m * m;
                    let (la, lb) = ((ra - m * rb) / den, (rb - m * ra) / den);
                    sub(x, add(scale(*na, la), scale(*nb, lb)))
                }
            }
            [(qa, na, _), (qb, nb, _), (qc, nc, _)] => {
                // The three planes' common point (Cramer's rule).
                let d = dot(*na, cross(*nb, *nc));
                if d.abs() < TANGENT_SIN * TANGENT_SIN {
                    alternate(g, sets, x)
                } else {
                    let (ha, hb, hc) = (dot(*na, *qa), dot(*nb, *qb), dot(*nc, *qc));
                    scale(add(add(scale(cross(*nb, *nc), ha), scale(cross(*nc, *na), hb)), scale(cross(*na, *nb), hc)), 1.0 / d)
                }
            }
            _ => return x,
        };
        if !next.iter().all(|v| v.is_finite()) {
            return x;
        }
        let step = dist(next, x);
        x = next;
        if step <= tol {
            break;
        }
    }
    x
}

/// One round of alternating projections onto each target.
fn alternate(g: &dyn Geometry, sets: &[&[u32]], p: P) -> P {
    let mut x = p;
    for s in sets {
        if let Some((q, _, _)) = on_target(g, s, x) {
            x = q;
        }
    }
    x
}
