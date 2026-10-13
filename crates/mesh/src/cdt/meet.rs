//! Whether two closed simplices meet properly: their intersection is exactly the simplex spanned
//! by their shared vertices (empty when they share none). The gift-wrapping step of the facet
//! recovery accepts a new tetrahedron only if it meets every cavity and front triangle properly
//! (Diazzi et al. 2023, §4.4 condition ii and App. B), and it must be exact on the coplanar
//! configurations CAD boundaries are made of.
//!
//! # Method
//! Two closed convex polytopes `A`, `B` whose relative interiors are disjoint are weakly separated
//! by a hyperplane `H` (`A` on one closed side, `B` on the other), and then `A ∩ B = (A ∩ H) ∩
//! (B ∩ H)`. The separating hyperplanes form a pointed polyhedral cone (in `(n, c)` space) whose
//! extreme rays are hyperplanes through `d` affinely independent points of `A ∪ B` (`d` the
//! dimension), so trying the hyperplanes through `d` of the vertices finds one whenever one
//! exists. When none exists the relative interiors overlap and the intersection is not a common
//! face. When one exists, every one gives the same `A ∩ B`, and the question recurses into `H`
//! (one dimension lower) with the vertices on it; a point is the base case. Before that, if all
//! of one simplex's vertices are shared, that simplex is a face of the other and they meet
//! properly.
//!
//! Every test is an exact predicate: `orient3d` in space, `orient2d` projected along an axis the
//! plane is not parallel to, and coordinate comparison along an axis the line is not
//! perpendicular to.

use crate::predicates::{Axis, GenericPoint, Sign, generic_compare, generic_orient2d, generic_orient3d};
use crate::{Error, Result};

use super::complex::face_toward;

#[derive(Debug, Clone, Copy)]
enum Level {
    Space,
    /// A plane, compared by orientation in its projection along this axis.
    Plane(Axis),
    /// A line, compared along this axis.
    Line(Axis),
    Point,
}

fn pt(p: &[GenericPoint], v: u32) -> Result<&GenericPoint> {
    p.get(v as usize).ok_or_else(|| Error::Failed("intersection test: vertex id out of range".into()))
}

/// Whether the closed simplices with vertex ids `a` and `b` (each affinely independent, ids of
/// distinct points) intersect exactly in the hull of their shared vertices.
pub(super) fn meet_properly(p: &[GenericPoint], a: &[u32], b: &[u32]) -> Result<bool> {
    proper(p, Level::Space, a, b)
}

/// Whether `x` is in the closed positive tetrahedron `t` (Diazzi et al. 2023, Alg. 3).
pub(super) fn in_closed_tet(p: &[GenericPoint], t: [u32; 4], x: u32) -> Result<bool> {
    for k in 0..4 {
        let [u, v, w] = face_toward(t, k);
        // The tetrahedron is on the face's positive side (tet4 volume, i.e. orient3d negative).
        if generic_orient3d(pt(p, u)?, pt(p, v)?, pt(p, w)?, pt(p, x)?) == Sign::Positive {
            return Ok(false);
        }
    }
    Ok(true)
}

fn subset(x: &[u32], y: &[u32]) -> bool {
    x.iter().all(|v| y.contains(v))
}

fn proper(p: &[GenericPoint], level: Level, a: &[u32], b: &[u32]) -> Result<bool> {
    if subset(a, b) || subset(b, a) {
        return Ok(true);
    }
    // Beyond the subset rule, a point holds one vertex, which both sides would share.
    let k = match level {
        Level::Space => 3,
        Level::Plane(_) => 2,
        Level::Line(_) => 1,
        Level::Point => return Ok(true),
    };
    let mut all: Vec<u32> = a.to_vec();
    all.extend(b.iter().filter(|v| !a.contains(v)));
    // Candidate hyperplanes through k of the points, those spanned by one simplex first (they
    // separate most pairs).
    let mut candidates: Vec<[u32; 3]> = Vec::new();
    combinations(&all, k, &mut [0; 3], 0, 0, &mut candidates);
    candidates.sort_by_key(|h| !(subset(&h[..k], a) || subset(&h[..k], b)));
    let mut spans = false;
    for h in &candidates {
        let h = &h[..k];
        if let (Level::Space, &[u, v, w]) = (level, h)
            && plane_axis(p, [u, v, w])?.is_none()
        {
            continue; // collinear: no plane
        }
        spans = true;
        let mut signs: Vec<(u32, Sign)> = Vec::with_capacity(all.len());
        for &x in &all {
            signs.push((x, if h.contains(&x) { Sign::Zero } else { side(p, level, h, x)? }));
        }
        let sign_of = |v: &u32| signs.iter().find(|(x, _)| x == v).map_or(Sign::Zero, |(_, s)| *s);
        let has = |set: &[u32], s: Sign| set.iter().any(|v| sign_of(v) == s);
        let separated = (!has(a, Sign::Negative) && !has(b, Sign::Positive)) || (!has(a, Sign::Positive) && !has(b, Sign::Negative));
        if !separated {
            continue;
        }
        let on = |set: &[u32]| -> Vec<u32> { set.iter().copied().filter(|v| sign_of(v) == Sign::Zero).collect() };
        let lower = match (level, h) {
            (Level::Space, &[u, v, w]) => {
                Level::Plane(plane_axis(p, [u, v, w])?.ok_or_else(|| Error::Failed("intersection test: degenerate plane".into()))?)
            }
            (Level::Plane(_), &[u, v]) => Level::Line(line_axis(p, u, v)?),
            _ => Level::Point,
        };
        return proper(p, lower, &on(a), &on(b));
    }
    if !spans && let (Level::Space, &[u, v, ..]) = (level, all.as_slice()) {
        // All points collinear: compare along their line.
        return proper(p, Level::Line(line_axis(p, u, v)?), a, b);
    }
    Ok(false)
}

/// An axis the plane through `u v w` is not parallel to (`None`: the points are collinear).
fn plane_axis(p: &[GenericPoint], [u, v, w]: [u32; 3]) -> Result<Option<Axis>> {
    let (a, b, c) = (pt(p, u)?, pt(p, v)?, pt(p, w)?);
    Ok(Axis::ALL.into_iter().find(|&ax| !generic_orient2d(a, b, c, ax).is_zero()))
}

/// An axis along which two distinct points differ.
fn line_axis(p: &[GenericPoint], u: u32, v: u32) -> Result<Axis> {
    let (a, b) = (pt(p, u)?, pt(p, v)?);
    Axis::ALL
        .into_iter()
        .find(|&ax| generic_compare(a, b, ax).is_ne())
        .ok_or_else(|| Error::Failed("intersection test: two vertices are the same point".into()))
}

fn side(p: &[GenericPoint], level: Level, h: &[u32], x: u32) -> Result<Sign> {
    let px = pt(p, x)?;
    Ok(match (level, h) {
        (Level::Space, &[u, v, w]) => generic_orient3d(pt(p, u)?, pt(p, v)?, pt(p, w)?, px),
        (Level::Plane(ax), &[u, v]) => generic_orient2d(pt(p, u)?, pt(p, v)?, px, ax),
        (Level::Line(ax), &[u]) => match generic_compare(px, pt(p, u)?, ax) {
            std::cmp::Ordering::Less => Sign::Negative,
            std::cmp::Ordering::Equal => Sign::Zero,
            std::cmp::Ordering::Greater => Sign::Positive,
        },
        _ => return Err(Error::Failed("intersection test: malformed hyperplane".into())),
    })
}

/// The `k`-subsets of `all` (`k ≤ 3`), each in the first `k` slots.
fn combinations(all: &[u32], k: usize, cur: &mut [u32; 3], len: usize, from: usize, out: &mut Vec<[u32; 3]>) {
    if len == k {
        out.push(*cur);
        return;
    }
    for i in from..all.len() {
        if let (Some(&v), Some(slot)) = (all.get(i), cur.get_mut(len)) {
            *slot = v;
            combinations(all, k, cur, len + 1, i + 1, out);
        }
    }
}
