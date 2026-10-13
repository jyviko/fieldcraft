//! Exact triangle–triangle intersection on the orientation predicates.
//!
//! Every decision is a sign of [`orient3d`] or of [`orient2d`] on a coordinate-plane projection,
//! so the answers are exact for every admissible input (Shewchuk 1997; the orientation-only
//! formulation follows Guigue and Devillers 2003). Triangles are closed sets.
//!
//! Two closed triangles meet exactly when an edge of one meets the other: their intersection is
//! a convex set whose extreme points lie on the boundary of one of them. Two triangles of a
//! surface may meet only in what they share, which [`surface_pair_intersects`] tests:
//! - **No shared vertex:** any contact is a defect (the six edge–triangle tests).
//! - **One shared vertex `v`:** the intersection is a convex set containing `v`; it holds another
//!   point exactly when an edge opposite `v` meets the other triangle, or an edge from `v` runs
//!   into the other triangle's corner at `v` ([`edge_enters_corner`]).
//! - **One shared edge:** the triangles meet beyond it only when they are coplanar and on the
//!   same side of it (folded onto each other).

use crate::Result;
use crate::predicates::{Axis, Point2, Point3, Sign, orient2d, orient3d};

/// The projection of `p` seen from the positive side of `drop`.
fn project(p: &Point3, drop: Axis) -> Result<Point2> {
    let (i, j) = drop.projected();
    let c = p.coords();
    Point2::new(c.get(i).copied().unwrap_or(0.0), c.get(j).copied().unwrap_or(0.0))
}

fn orient2d_on(a: &Point3, b: &Point3, c: &Point3, drop: Axis) -> Result<Sign> {
    Ok(orient2d(&project(a, drop)?, &project(b, drop)?, &project(c, drop)?))
}

/// A coordinate plane on which triangle `abc` projects to a proper triangle, and the
/// orientation of that projection; `None` when the corners are collinear.
fn plane_axis([a, b, c]: [&Point3; 3]) -> Result<Option<(Axis, Sign)>> {
    for axis in [Axis::Z, Axis::X, Axis::Y] {
        let s = orient2d_on(a, b, c, axis)?;
        if !s.is_zero() {
            return Ok(Some((axis, s)));
        }
    }
    Ok(None)
}

/// Whether the corners are exactly collinear (or coincide).
pub fn is_degenerate(t: [&Point3; 3]) -> Result<bool> {
    Ok(plane_axis(t)?.is_none())
}

/// Closed point-in-triangle test in the plane of `t` (`p` must lie in it).
fn point_in_triangle(p: &Point3, [a, b, c]: [&Point3; 3], drop: Axis, s: Sign) -> Result<bool> {
    for (u, v) in [(a, b), (b, c), (c, a)] {
        if orient2d_on(u, v, p, drop)? == -s {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `r`, collinear with `p` and `q`, lies on the closed segment `pq`.
fn on_segment(p: &Point3, q: &Point3, r: &Point3) -> bool {
    let (p, q, r) = (p.coords(), q.coords(), r.coords());
    (0..3).all(|k| {
        let (lo, hi) = if p[k] <= q[k] { (p[k], q[k]) } else { (q[k], p[k]) };
        lo <= r[k] && r[k] <= hi
    })
}

/// Closed segment–segment intersection of two coplanar segments, on the projection `drop`
/// (which must be injective on their plane).
fn segments_meet(p: &Point3, q: &Point3, a: &Point3, b: &Point3, drop: Axis) -> Result<bool> {
    let d1 = orient2d_on(p, q, a, drop)?;
    let d2 = orient2d_on(p, q, b, drop)?;
    let d3 = orient2d_on(a, b, p, drop)?;
    let d4 = orient2d_on(a, b, q, drop)?;
    if d1 * d2 == Sign::Negative && d3 * d4 == Sign::Negative {
        return Ok(true);
    }
    Ok((d1.is_zero() && on_segment(p, q, a))
        || (d2.is_zero() && on_segment(p, q, b))
        || (d3.is_zero() && on_segment(a, b, p))
        || (d4.is_zero() && on_segment(a, b, q)))
}

/// Whether the closed segment `pq` meets the closed, non-degenerate triangle `t`.
pub fn segment_meets_triangle(p: &Point3, q: &Point3, t: [&Point3; 3]) -> Result<bool> {
    let [a, b, c] = t;
    let op = orient3d(a, b, c, p);
    let oq = orient3d(a, b, c, q);
    if op == oq && !op.is_zero() {
        return Ok(false);
    }
    if op.is_zero() && oq.is_zero() {
        let Some((drop, s)) = plane_axis(t)? else { return Ok(false) };
        if point_in_triangle(p, t, drop, s)? || point_in_triangle(q, t, drop, s)? {
            return Ok(true);
        }
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if segments_meet(p, q, u, v, drop)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    // The segment crosses or touches the plane at one point, which is inside the triangle
    // exactly when the line pq passes on no strict outside of its three edges.
    let s = [orient3d(p, q, a, b), orient3d(p, q, b, c), orient3d(p, q, c, a)];
    Ok(!(s.contains(&Sign::Positive) && s.contains(&Sign::Negative)))
}

/// Whether two closed, non-degenerate triangles intersect at all.
pub fn triangles_meet(s: [&Point3; 3], t: [&Point3; 3]) -> Result<bool> {
    for (p, q) in [(s[0], s[1]), (s[1], s[2]), (s[2], s[0])] {
        if segment_meets_triangle(p, q, t)? {
            return Ok(true);
        }
    }
    for (p, q) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
        if segment_meets_triangle(p, q, s)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// For triangles sharing the corner `v`: whether the edge from `v` to `a` contains a point of
/// triangle `v c d` other than `v` (it lies in that triangle's plane, inside its corner at `v`).
fn edge_enters_corner(v: &Point3, a: &Point3, c: &Point3, d: &Point3) -> Result<bool> {
    if !orient3d(v, c, d, a).is_zero() {
        return Ok(false);
    }
    let Some((drop, s)) = plane_axis([v, c, d])? else { return Ok(false) };
    Ok(orient2d_on(v, c, a, drop)? != -s && orient2d_on(v, a, d, drop)? != -s)
}

/// Whether two triangles of a surface, given by vertex ids and points, meet anywhere other than
/// in the vertices and edge they share. Both must be non-degenerate.
pub fn surface_pair_intersects(ids: [[u32; 3]; 2], s: [&Point3; 3], t: [&Point3; 3]) -> Result<bool> {
    let [si, ti] = ids;
    // Positions in s of the corners shared with t, and their positions in t.
    let mut shared = [(0usize, 0usize); 3];
    let mut n = 0;
    for (i, v) in si.iter().enumerate() {
        if let (Some(j), Some(slot)) = (ti.iter().position(|w| w == v), shared.get_mut(n)) {
            *slot = (i, j);
            n += 1;
        }
    }
    let shared = shared.get(..n).unwrap_or(&[]);
    match shared {
        [] => triangles_meet(s, t),
        &[(i, j)] => {
            let v = s[i];
            let (a, b) = (s[(i + 1) % 3], s[(i + 2) % 3]);
            let (c, d) = (t[(j + 1) % 3], t[(j + 2) % 3]);
            Ok(segment_meets_triangle(a, b, t)?
                || segment_meets_triangle(c, d, s)?
                || edge_enters_corner(v, a, c, d)?
                || edge_enters_corner(v, b, c, d)?
                || edge_enters_corner(v, c, a, b)?
                || edge_enters_corner(v, d, a, b)?)
        }
        &[(i0, _), (i1, _)] => {
            // The corner of each triangle off the shared edge.
            let k = 3 - i0 - i1;
            let Some(l) = (0..3).find(|j| !shared.iter().any(|&(_, sj)| sj == *j)) else { return Ok(true) };
            let (u, w, a, b) = (s[i0], s[i1], s[k], t[l]);
            if !orient3d(u, w, a, b).is_zero() {
                return Ok(false);
            }
            let Some((drop, _)) = plane_axis(s)? else { return Ok(false) };
            Ok(orient2d_on(u, w, a, drop)? == orient2d_on(u, w, b, drop)?)
        }
        // The same three vertices: a duplicate triangle.
        _ => Ok(true),
    }
}
