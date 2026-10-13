//! Exact triangle–triangle intersection on the orientation predicates.
//!
//! Every decision is a sign of [`orient3d`] or of [`orient2d_projected`] on a coordinate-plane
//! projection, so the answers are exact for every admissible input (Shewchuk 1997; the
//! orientation-only formulation follows Guigue and Devillers 2003). Triangles are closed sets.
//!
//! Two closed triangles meet exactly when an edge of one meets the other: their intersection is
//! a convex set whose extreme points lie on the boundary of one of them. Two triangles of a
//! surface may meet only in what they share, which [`surface_pair_intersects`] tests:
//! - **No shared vertex:** any contact is a defect ([`triangles_meet`]).
//! - **One shared vertex `v`:** the intersection is a convex set containing `v`; it holds another
//!   point exactly when an edge opposite `v` meets the other triangle, or an edge from `v` runs
//!   into the other triangle's corner at `v`.
//! - **One shared edge:** the triangles meet beyond it only when they are coplanar and on the
//!   same side of it (folded onto each other).
//!
//! Every vertex's side of the other triangle's plane is computed once per pair; a triangle
//! strictly on one side of the other's plane rejects the pair at once.

use crate::predicates::{Axis, Point3, Sign, orient2d_projected, orient3d};

type Tri<'a> = [&'a Point3; 3];

/// A coordinate plane on which triangle `abc` projects to a proper triangle, and the
/// orientation of that projection; `None` when the corners are collinear.
fn plane_axis([a, b, c]: Tri) -> Option<(Axis, Sign)> {
    [Axis::Z, Axis::X, Axis::Y].into_iter().find_map(|axis| {
        let s = orient2d_projected(a, b, c, axis);
        (!s.is_zero()).then_some((axis, s))
    })
}

/// Whether the corners are exactly collinear (or coincide).
pub fn is_degenerate(t: Tri) -> bool {
    plane_axis(t).is_none()
}

fn edges<'a>([a, b, c]: Tri<'a>) -> [(&'a Point3, &'a Point3); 3] {
    [(a, b), (b, c), (c, a)]
}

/// The side of `p` against the plane of `t`.
fn side(t: Tri, p: &Point3) -> Sign {
    orient3d(t[0], t[1], t[2], p)
}

/// Closed point-in-triangle test in the plane of `t` (`p` must lie in it).
fn point_in_triangle(p: &Point3, t: Tri, drop: Axis, s: Sign) -> bool {
    edges(t).into_iter().all(|(u, v)| orient2d_projected(u, v, p, drop) != -s)
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
fn segments_meet(p: &Point3, q: &Point3, a: &Point3, b: &Point3, drop: Axis) -> bool {
    let d1 = orient2d_projected(p, q, a, drop);
    let d2 = orient2d_projected(p, q, b, drop);
    let d3 = orient2d_projected(a, b, p, drop);
    let d4 = orient2d_projected(a, b, q, drop);
    (d1 * d2 == Sign::Negative && d3 * d4 == Sign::Negative)
        || (d1.is_zero() && on_segment(p, q, a))
        || (d2.is_zero() && on_segment(p, q, b))
        || (d3.is_zero() && on_segment(a, b, p))
        || (d4.is_zero() && on_segment(a, b, q))
}

/// Whether the closed segment `pq` meets the closed, non-degenerate triangle `t`, given the
/// sides `op`, `oq` of `p` and `q` against `t`'s plane.
fn segment_meets(p: &Point3, q: &Point3, op: Sign, oq: Sign, t: Tri) -> bool {
    if op == oq && !op.is_zero() {
        return false;
    }
    if op.is_zero() && oq.is_zero() {
        let Some((drop, s)) = plane_axis(t) else { return false };
        return point_in_triangle(p, t, drop, s)
            || point_in_triangle(q, t, drop, s)
            || edges(t).into_iter().any(|(u, v)| segments_meet(p, q, u, v, drop));
    }
    // The segment crosses or touches the plane at one point, which is inside the triangle
    // exactly when the line pq passes on no strict outside of its three edges.
    let s = edges(t).map(|(u, v)| orient3d(p, q, u, v));
    !(s.contains(&Sign::Positive) && s.contains(&Sign::Negative))
}

/// Whether the closed segment `pq` meets the closed, non-degenerate triangle `t`.
pub fn segment_meets_triangle(p: &Point3, q: &Point3, t: Tri) -> bool {
    segment_meets(p, q, side(t, p), side(t, q), t)
}

/// Whether two closed, non-degenerate triangles intersect at all.
pub fn triangles_meet(s: Tri, t: Tri) -> bool {
    let s_sides = s.map(|p| side(t, p));
    let t_sides = t.map(|p| side(s, p));
    let one_side = |x: [Sign; 3]| !x[0].is_zero() && x[0] == x[1] && x[1] == x[2];
    if one_side(s_sides) || one_side(t_sides) {
        return false;
    }
    let crosses = |a: Tri, o: [Sign; 3], b: Tri| (0..3).any(|k| segment_meets(a[k], a[(k + 1) % 3], o[k], o[(k + 1) % 3], b));
    crosses(s, s_sides, t) || crosses(t, t_sides, s)
}

/// For triangles sharing the corner `v`: whether the edge from `v` to `a` (whose side of the
/// plane of triangle `v c d` is `side_a`) contains a point of that triangle other than `v`: it
/// lies in that plane, inside the triangle's corner at `v`.
fn edge_enters_corner(v: &Point3, a: &Point3, side_a: Sign, c: &Point3, d: &Point3) -> bool {
    if !side_a.is_zero() {
        return false;
    }
    let Some((drop, s)) = plane_axis([v, c, d]) else { return false };
    orient2d_projected(v, c, a, drop) != -s && orient2d_projected(v, a, d, drop) != -s
}

/// Whether two triangles of a surface, given by vertex ids and points, meet anywhere other than
/// in the vertices and edge they share. Both must be non-degenerate.
pub fn surface_pair_intersects(ids: [[u32; 3]; 2], s: Tri, t: Tri) -> bool {
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
    match shared.get(..n).unwrap_or(&[]) {
        [] => triangles_meet(s, t),
        &[(i, j)] => {
            let v = s[i];
            let (a, b) = (s[(i + 1) % 3], s[(i + 2) % 3]);
            let (c, d) = (t[(j + 1) % 3], t[(j + 2) % 3]);
            let (oa, ob, oc, od) = (side(t, a), side(t, b), side(s, c), side(s, d));
            segment_meets(a, b, oa, ob, t)
                || segment_meets(c, d, oc, od, s)
                || edge_enters_corner(v, a, oa, c, d)
                || edge_enters_corner(v, b, ob, c, d)
                || edge_enters_corner(v, c, oc, a, b)
                || edge_enters_corner(v, d, od, a, b)
        }
        &[(i0, j0), (i1, j1)] => {
            // The corner of each triangle off the shared edge.
            let (u, w, a, b) = (s[i0], s[i1], s[3 - i0 - i1], t[3 - j0 - j1]);
            if !orient3d(u, w, a, b).is_zero() {
                return false;
            }
            let Some((drop, _)) = plane_axis(s) else { return false };
            orient2d_projected(u, w, a, drop) == orient2d_projected(u, w, b, drop)
        }
        // The same three vertices: a duplicate triangle.
        _ => true,
    }
}
