//! Shape metrics from corner positions and corner Jacobians.
//!
//! - Tetrahedra: Shewchuk, *What is a good linear finite element? Interpolation, conditioning,
//!   anisotropy, and quality measures*, 2002 (dihedral angles, radius ratio, aspect ratio), and
//!   the volume–length measure Klingner and Shewchuk recommend for mesh improvement (IMR 2007, §2).
//! - Hexahedra, wedges and quadrilaterals: Knupp's algebraic metrics built on the corner Jacobian
//!   matrices (Knupp, *Algebraic mesh quality metrics*, SIAM J. Sci. Comput. 23(1), 2001): the
//!   scaled Jacobian `det(A) / (|a1| |a2| |a3|)` and the shape metric `3 det(A)^{2/3} / |A|²_F`.
//! - Quadrilateral aspect ratio and skew from the principal axes `X1 = (p1 − p0) + (p2 − p3)`,
//!   `X2 = (p2 − p1) + (p3 − p0)` (Robinson, *CRE method of element testing and the Jacobian
//!   shape parameters*, Eng. Comput. 4(2):113–118, 1987): aspect `max(|X1|, |X2|) / min(…)`, skew
//!   `|X1 · X2| / (|X1| |X2|)`.
//!
//! Every metric is defined for degenerate input (no division by zero, no NaN): a degenerate
//! element scores 0 on bounded metrics and `+∞` on aspect and edge ratios.

use std::f64::consts::SQRT_2;

use super::Metric;

use crate::vec3::angle_deg as angle;
pub(crate) use crate::vec3::{P, cross, det, dot, norm, sub};

/// `num / den`, or `fallback` when the denominator is zero or the quotient is not finite.
fn ratio(num: f64, den: f64, fallback: f64) -> f64 {
    let r = num / den;
    if den != 0.0 && r.is_finite() { r } else { fallback }
}

/// Normalized scaled Jacobian of one corner: the determinant of its three unit edge vectors.
fn corner_det(a: P, b: P, c: P) -> f64 {
    ratio(det(a, b, c), norm(a) * norm(b) * norm(c), 0.0)
}

/// Largest over smallest edge length.
pub(crate) fn edge_ratio(p: &[P], edges: &[(usize, usize)]) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, 0.0_f64);
    for &(a, b) in edges {
        if let (Some(&x), Some(&y)) = (p.get(a), p.get(b)) {
            let l = norm(sub(x, y));
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    ratio(hi, lo, f64::INFINITY).max(1.0)
}

/// Tetrahedron metrics on corners `p` (positive orientation as in [`super::jacobian`]).
pub(crate) fn tet(p: [P; 4]) -> Vec<(Metric, f64)> {
    let [a, b, c, d] = p;
    let volume = det(sub(b, a), sub(c, a), sub(d, a)) / 6.0;
    let edges = super::jacobian::TET_EDGES.map(|(i, j)| norm(sub(p[j], p[i])));
    let faces =
        [cross(sub(c, b), sub(d, b)), cross(sub(d, a), sub(c, a)), cross(sub(b, a), sub(d, a)), cross(sub(c, a), sub(b, a))].map(|n| 0.5 * norm(n));
    let area: f64 = faces.iter().sum();
    let l_max = edges.iter().fold(0.0_f64, |m, &l| m.max(l));
    let a_max = faces.iter().fold(0.0_f64, |m, &l| m.max(l));
    let l_rms = (edges.iter().map(|l| l * l).sum::<f64>() / 6.0).sqrt();

    // Dihedral angle at edge (i, j): the angle between the other two vertices as seen
    // perpendicular to the edge.
    let mut dihedral = (180.0_f64, 0.0_f64);
    // The two vertices off each edge of TET_EDGES, in the same order.
    let opposite = [(2, 3), (0, 3), (1, 3), (1, 2), (0, 2), (0, 1)];
    for ((i, j), (k, l)) in super::jacobian::TET_EDGES.into_iter().zip(opposite) {
        let e = sub(p[j], p[i]);
        let ee = dot(e, e);
        let perp = |v: P| {
            let w = sub(v, p[i]);
            let t = ratio(dot(w, e), ee, 0.0);
            sub(w, [t * e[0], t * e[1], t * e[2]])
        };
        let th = angle(perp(p[k]), perp(p[l]));
        dihedral = (dihedral.0.min(th), dihedral.1.max(th));
    }

    // Circumradius R = |num| / (12 |V|) with num = |a|²(b×c) + |b|²(c×a) + |c|²(a×b) for edge
    // vectors a, b, c from one vertex; inradius r = 3V / area; radius ratio 3r/R.
    let (u, v, w) = (sub(b, a), sub(c, a), sub(d, a));
    let (bc, ca, ab) = (cross(v, w), cross(w, u), cross(u, v));
    let (lu, lv, lw) = (dot(u, u), dot(v, v), dot(w, w));
    let num = norm([lu * bc[0] + lv * ca[0] + lw * ab[0], lu * bc[1] + lv * ca[1] + lw * ab[1], lu * bc[2] + lv * ca[2] + lw * ab[2]]);
    let radius_ratio = ratio(108.0 * volume * volume.abs(), area * num, 0.0);

    // Smallest altitude 3|V| / largest face; normalized so the regular tetrahedron scores 1.
    let aspect = ratio((2.0_f64 / 3.0).sqrt() * l_max * a_max, 3.0 * volume.abs(), f64::INFINITY);
    let volume_length = ratio(6.0 * SQRT_2 * volume, l_rms.powi(3), 0.0);

    // Scaled Jacobian: 6V over the largest product of the three edge lengths at a corner, × √2.
    let corner = [(0, 1, 2, 3), (1, 2, 0, 3), (2, 0, 1, 3), (3, 0, 2, 1)]
        .map(|(o, x, y, z)| norm(sub(p[x], p[o])) * norm(sub(p[y], p[o])) * norm(sub(p[z], p[o])))
        .iter()
        .fold(0.0_f64, |m, &x| m.max(x));
    let scaled = ratio(SQRT_2 * 6.0 * volume, corner, 0.0);

    vec![
        (Metric::ScaledJacobian, scaled),
        (Metric::MinDihedral, dihedral.0),
        (Metric::MaxDihedral, dihedral.1),
        (Metric::RadiusRatio, radius_ratio),
        (Metric::AspectRatio, aspect.max(1.0)),
        (Metric::VolumeLength, volume_length),
    ]
}

/// Triangle area and metrics.
pub(crate) fn tri(p: [P; 3]) -> (f64, Vec<(Metric, f64)>) {
    let area = 0.5 * norm(cross(sub(p[1], p[0]), sub(p[2], p[0])));
    let angles = [0, 1, 2].map(|i| angle(sub(p[(i + 1) % 3], p[i]), sub(p[(i + 2) % 3], p[i])));
    let l_max = (0..3).map(|i| norm(sub(p[(i + 1) % 3], p[i]))).fold(0.0_f64, f64::max);
    // Longest edge over smallest altitude, normalized so the equilateral triangle scores 1.
    let aspect = ratio(3.0_f64.sqrt() * l_max * l_max, 4.0 * area, f64::INFINITY).max(1.0);
    (
        area,
        vec![
            (Metric::MinAngle, angles.iter().fold(180.0, |m: f64, &a| m.min(a))),
            (Metric::MaxAngle, angles.iter().fold(0.0, |m: f64, &a| m.max(a))),
            (Metric::AspectRatio, aspect),
        ],
    )
}

/// Quadrilateral area (of its projection on the mean plane) and metrics. The scaled Jacobian of
/// a corner is measured against the normal of the diagonals, so a concave or bow-tie
/// quadrilateral scores ≤ 0.
pub(crate) fn quad(p: [P; 4]) -> (f64, Vec<(Metric, f64)>) {
    let n = cross(sub(p[2], p[0]), sub(p[3], p[1]));
    let area = 0.5 * norm(n);
    let mut scaled = f64::INFINITY;
    let mut angles = (180.0_f64, 0.0_f64);
    for i in 0..4 {
        let next = sub(p[(i + 1) % 4], p[i]);
        let prev = sub(p[(i + 3) % 4], p[i]);
        scaled = scaled.min(ratio(dot(cross(next, prev), n), norm(next) * norm(prev) * norm(n), 0.0));
        let a = angle(next, prev);
        angles = (angles.0.min(a), angles.1.max(a));
    }
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
    let (aspect, skew) = quad_axes(p);
    (
        area,
        vec![
            (Metric::ScaledJacobian, scaled),
            (Metric::MinAngle, angles.0),
            (Metric::MaxAngle, angles.1),
            (Metric::EdgeRatio, edge_ratio(&p, &edges)),
            (Metric::AspectRatio, aspect),
            (Metric::Skew, skew),
        ],
    )
}

/// Corner `i` of a planar quadrilateral: the cross product `(p[i+1] − p[i]) × (p[i−1] − p[i])`
/// (twice the corner triangle's signed area, positive for a left turn of a counterclockwise quad)
/// and that cross over the two edge lengths (the corner's scaled Jacobian, `None` when an edge
/// has zero length).
pub(crate) fn planar_corner(p: &[[f64; 2]; 4], i: usize) -> (f64, Option<f64>) {
    let (a, b, c) = (p[(i + 3) % 4], p[i % 4], p[(i + 1) % 4]);
    let (u, w) = ([c[0] - b[0], c[1] - b[1]], [a[0] - b[0], a[1] - b[1]]);
    let cross = u[0] * w[1] - u[1] * w[0];
    let den = (u[0] * u[0] + u[1] * u[1]).sqrt() * (w[0] * w[0] + w[1] * w[1]).sqrt();
    let scaled = cross / den;
    (cross, (den > 0.0 && scaled.is_finite()).then_some(scaled))
}

/// Robinson's aspect ratio and skew of a quadrilateral from its principal axes; `(+∞, 1)` when
/// an axis vanishes.
pub(crate) fn quad_axes(p: [P; 4]) -> (f64, f64) {
    let x1 = [p[1][0] - p[0][0] + p[2][0] - p[3][0], p[1][1] - p[0][1] + p[2][1] - p[3][1], p[1][2] - p[0][2] + p[2][2] - p[3][2]];
    let x2 = [p[2][0] - p[1][0] + p[3][0] - p[0][0], p[2][1] - p[1][1] + p[3][1] - p[0][1], p[2][2] - p[1][2] + p[3][2] - p[0][2]];
    let (l1, l2) = (norm(x1), norm(x2));
    let aspect = ratio(l1.max(l2), l1.min(l2), f64::INFINITY).max(1.0);
    let skew = ratio(dot(x1, x2).abs(), l1 * l2, 1.0).min(1.0);
    (aspect, skew)
}

/// Wedge scaled Jacobian: the smallest determinant of the three unit edge vectors at a corner,
/// taken in right-handed order (1 for a right prism on a right-angled corner, sin 60° ≈ 0.866 on
/// an equilateral triangle).
pub(crate) fn wedge_scaled_jacobian(p: [P; 6]) -> f64 {
    [(0, 1, 2, 3), (1, 2, 0, 4), (2, 0, 1, 5), (3, 5, 4, 0), (4, 3, 5, 1), (5, 4, 3, 2)]
        .map(|(o, a, b, c)| corner_det(sub(p[a], p[o]), sub(p[b], p[o]), sub(p[c], p[o])))
        .iter()
        .fold(f64::INFINITY, |m, &x| m.min(x))
}

/// Pyramid scaled Jacobian: the smallest determinant of the unit edge vectors at the four base
/// corners (to the next and previous base corners and to the apex, right-handed), divided by its
/// value √2/2 on the pyramid whose edges are all equal, so that pyramid scores 1.
pub(crate) fn pyramid_scaled_jacobian(p: [P; 5]) -> f64 {
    let worst = [(0, 1, 3), (1, 2, 0), (2, 3, 1), (3, 0, 2)]
        .map(|(o, a, b)| corner_det(sub(p[a], p[o]), sub(p[b], p[o]), sub(p[4], p[o])))
        .iter()
        .fold(f64::INFINITY, |m, &x| m.min(x));
    (worst / std::f64::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0)
}

/// Scaled Jacobian and Knupp shape of one Jacobian matrix `[a b c]` (hexahedron corners and centre).
pub(crate) fn jacobian_metrics(a: P, b: P, c: P) -> (f64, f64) {
    let d = det(a, b, c);
    let frob = dot(a, a) + dot(b, b) + dot(c, c);
    let shape = if d > 0.0 { ratio(3.0 * crate::vec3::cbrt(d).powi(2), frob, 0.0) } else { 0.0 };
    (corner_det(a, b, c), shape)
}
