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

#[derive(Debug, Clone, Copy)]
enum Level {
    Space,
    /// A plane, compared by orientation in its projection along this axis.
    Plane(Axis),
    /// A line, compared along this axis.
    Line(Axis),
}

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

fn pt(p: &[GenericPoint], v: u32) -> Result<&GenericPoint> {
    p.get(v as usize).ok_or_else(|| Error::Failed("intersection test: vertex id out of range".into()))
}

fn ordering_sign(o: std::cmp::Ordering) -> Sign {
    match o {
        std::cmp::Ordering::Less => Sign::Negative,
        std::cmp::Ordering::Equal => Sign::Zero,
        std::cmp::Ordering::Greater => Sign::Positive,
    }
}

/// Whether the closed simplices with vertex ids `a` and `b` (each affinely independent, ids of
/// distinct points) intersect exactly in the hull of their shared vertices.
pub(super) fn meet_properly(p: &[GenericPoint], a: &[u32], b: &[u32]) -> Result<bool> {
    proper(p, Level::Space, a, b)
}

/// Whether closed `x` is in the closed positive tetrahedron `t` (Diazzi et al. 2023, Alg. 3).
pub(super) fn in_closed_tet(p: &[GenericPoint], t: [u32; 4], x: u32) -> Result<bool> {
    let [a, b, c, d] = t;
    let faces = [[b, d, c], [a, c, d], [a, d, b], [a, b, c]];
    for [u, v, w] in faces {
        // Positive tet4 volume of (face, opposite) means orient3d negative; x must not be on
        // the other side.
        if -generic_orient3d(pt(p, u)?, pt(p, v)?, pt(p, w)?, pt(p, x)?) == Sign::Negative {
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
    let mut all: Vec<u32> = a.to_vec();
    for &v in b {
        if !all.contains(&v) {
            all.push(v);
        }
    }
    let k = match level {
        Level::Space => 3,
        Level::Plane(_) => 2,
        Level::Line(_) => 1,
    };
    // Candidate hyperplanes through k of the points, those spanned by one simplex first (they
    // separate most pairs).
    let mut candidates: Vec<Vec<u32>> = Vec::new();
    combinations(&all, k, &mut Vec::new(), 0, &mut candidates);
    candidates.sort_by_key(|h| !(subset(h, a) || subset(h, b)));
    let mut spans = false;
    for h in &candidates {
        let Some(lower) = hyperplane(p, level, h)? else { continue };
        spans = true;
        let mut signs = Vec::with_capacity(all.len());
        for &x in &all {
            signs.push(if h.contains(&x) { Sign::Zero } else { side(p, level, h, x)? });
        }
        let side_of = |set: &[u32]| -> (bool, bool) {
            let mut pos = false;
            let mut neg = false;
            for v in set {
                match all.iter().position(|u| u == v).and_then(|i| signs.get(i)) {
                    Some(Sign::Positive) => pos = true,
                    Some(Sign::Negative) => neg = true,
                    _ => {}
                }
            }
            (pos, neg)
        };
        let ((ap, an), (bp, bn)) = (side_of(a), side_of(b));
        if (an || bp) && (ap || bn) {
            continue; // not separating
        }
        let on = |set: &[u32]| -> Vec<u32> {
            set.iter().copied().filter(|v| all.iter().position(|u| u == v).and_then(|i| signs.get(i)) == Some(&Sign::Zero)).collect()
        };
        return proper_lower(p, lower, &on(a), &on(b));
    }
    if !spans {
        // Space with all points collinear: compare along the line.
        if let Level::Space = level {
            let axis = line_axis(p, &all)?;
            return proper(p, Level::Line(axis), a, b);
        }
    }
    Ok(false)
}

fn proper_lower(p: &[GenericPoint], lower: Option<Level>, a: &[u32], b: &[u32]) -> Result<bool> {
    match lower {
        Some(level) => proper(p, level, a, b),
        // A point: the vertices on it are that one point, so one side is empty or both are the
        // shared vertex (caught by the subset rule above).
        None => Ok(a.is_empty() || b.is_empty() || subset(a, b) || subset(b, a)),
    }
}

/// An axis along which two of the (distinct) points differ.
fn line_axis(p: &[GenericPoint], all: &[u32]) -> Result<Axis> {
    let (Some(&u), Some(&v)) = (all.first(), all.get(1)) else { return Ok(Axis::X) };
    for axis in AXES {
        if generic_compare(pt(p, u)?, pt(p, v)?, axis).is_ne() {
            return Ok(axis);
        }
    }
    Err(Error::Failed("intersection test: two vertices are the same point".into()))
}

/// For a non-degenerate hyperplane through `h`: the level inside it (`None` for a point), or
/// `Ok(None)` wrapped as `None` when `h` does not span a hyperplane.
fn hyperplane(p: &[GenericPoint], level: Level, h: &[u32]) -> Result<Option<Option<Level>>> {
    Ok(match (level, h) {
        (Level::Space, &[u, v, w]) => {
            let (a, b, c) = (pt(p, u)?, pt(p, v)?, pt(p, w)?);
            AXES.into_iter().find(|&ax| !generic_orient2d(a, b, c, ax).is_zero()).map(|ax| Some(Level::Plane(ax)))
        }
        (Level::Plane(_), &[u, v]) => Some(Some(Level::Line(line_axis(p, &[u, v])?))),
        (Level::Line(_), &[_]) => Some(None),
        _ => None,
    })
}

fn side(p: &[GenericPoint], level: Level, h: &[u32], x: u32) -> Result<Sign> {
    let px = pt(p, x)?;
    Ok(match (level, h) {
        (Level::Space, &[u, v, w]) => generic_orient3d(pt(p, u)?, pt(p, v)?, pt(p, w)?, px),
        (Level::Plane(ax), &[u, v]) => generic_orient2d(pt(p, u)?, pt(p, v)?, px, ax),
        (Level::Line(ax), &[u]) => ordering_sign(generic_compare(px, pt(p, u)?, ax)),
        _ => return Err(Error::Failed("intersection test: malformed hyperplane".into())),
    })
}

fn combinations(all: &[u32], k: usize, cur: &mut Vec<u32>, from: usize, out: &mut Vec<Vec<u32>>) {
    if cur.len() == k {
        out.push(cur.clone());
        return;
    }
    for i in from..all.len() {
        if let Some(&v) = all.get(i) {
            cur.push(v);
            combinations(all, k, cur, i + 1, out);
            cur.pop();
        }
    }
}
