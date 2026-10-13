//! Indirect predicates on implicit points (Attene 2020, §4–§5), from the paper only.
//!
//! An implicit point is kept as the explicit points that define it and is never rounded: a
//! line–plane intersection ([`Lpi`], §4.2) or a three-plane intersection ([`Tpi`]). Each has
//! homogeneous coordinates `(X, W)` that are polynomials in the defining coordinates, so a
//! predicate on implicit points is the sign of a polynomial times the signs of the `W`s
//! (Attene §4.3–§4.6, §5.1: "count how many of them have a negative sign").
//!
//! Evaluation follows Attene §5: the interval filter (§5.2, with the defining points' `(X, W)`
//! cached as intervals, §5.4), then exact evaluation. Two deliberate differences: there is no
//! semi-static floating-point stage yet (it needs a per-formula error bound; the interval stage
//! is the dynamic equivalent), and the exact stage uses [`super::dyadic::Dyadic`] integers
//! instead of expansions, which could underflow at these polynomial degrees. All-explicit
//! inputs go straight to Shewchuk's predicates.
//!
//! A construction is accepted only when its `W` is exactly nonzero (the line is not parallel to
//! the plane, the three planes meet in one point), so the predicates are total.

use std::cmp::Ordering;

use super::dyadic::Dyadic;
use super::interval::Interval;
use super::ring::{Ring, Signed, det2, det3};
use super::{Point2, Point3, Sign, orient2d, orient3d};
use crate::{Error, Result};

/// A coordinate axis: the axis compared by [`generic_compare`], or dropped by
/// [`generic_orient2d`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub(crate) fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// The two remaining axes in cyclic order (`X → (y, z)`, `Y → (z, x)`, `Z → (x, y)`), so
    /// the projection is seen from the positive side of the dropped axis.
    pub(crate) fn projected(self) -> (usize, usize) {
        match self {
            Axis::X => (1, 2),
            Axis::Y => (2, 0),
            Axis::Z => (0, 1),
        }
    }
}

/// Homogeneous coordinates `X / W`; `w: None` means `W = 1` (an explicit point), which spares
/// the filter needless multiplications.
#[derive(Debug, Clone)]
struct Hom<T> {
    x: [T; 3],
    w: Option<T>,
}

fn lift<T: Ring>(p: &[f64; 3]) -> [T; 3] {
    p.map(T::from_f64)
}

fn sub3<T: Ring>(a: &[T; 3], b: &[T; 3]) -> [T; 3] {
    let ([a0, a1, a2], [b0, b1, b2]) = (a, b);
    [a0.clone() - b0.clone(), a1.clone() - b1.clone(), a2.clone() - b2.clone()]
}

fn coord<T: Clone>(v: &[T; 3], k: usize) -> T {
    let [x, y, z] = v;
    match k {
        0 => x.clone(),
        1 => y.clone(),
        _ => z.clone(),
    }
}

/// `v · w`, or `v` when `w = 1`.
fn mulw<T: Ring>(v: T, w: &Option<T>) -> T {
    match w {
        None => v,
        Some(w) => v * w.clone(),
    }
}

/// The sign of `W` (positive for explicit points), `None` if uncertain.
fn w_sign<T: Signed>(w: &Option<T>) -> Option<Sign> {
    w.as_ref().map_or(Some(Sign::Positive), Signed::sign)
}

/// Line `q1 q2` meets plane `r s t` (Attene §4.2): with `d = det[q1 - q2; s - r; t - r]` and
/// `n = det[q1 - r; s - r; t - r]`, the point is `(d·q1 + n·(q2 - q1)) / d`.
fn lpi_hom<T: Ring>([q1, q2, r, s, t]: &[[f64; 3]; 5]) -> Hom<T> {
    let (q1, q2, r, s, t) = (lift::<T>(q1), lift::<T>(q2), lift::<T>(r), lift::<T>(s), lift::<T>(t));
    let (sr, tr) = (sub3(&s, &r), sub3(&t, &r));
    let d = det3(&[sub3(&q1, &q2), sr.clone(), tr.clone()]);
    let n = det3(&[sub3(&q1, &r), sr, tr]);
    let dir = sub3(&q2, &q1);
    let x = std::array::from_fn(|k| d.clone() * coord(&q1, k) + n.clone() * coord(&dir, k));
    Hom { x, w: Some(d) }
}

/// Three planes `(r, s, t)` meet (Cramer's rule): normals `N_i = (s_i - r_i) × (t_i - r_i)`,
/// offsets `c_i = N_i · r_i`, `W = det[N_1; N_2; N_3]`, `X_k` = the same determinant with
/// column `k` replaced by `c`.
fn tpi_hom<T: Ring>(planes: &[[[f64; 3]; 3]; 3]) -> Hom<T> {
    let plane = |[r, s, t]: &[[f64; 3]; 3]| -> ([T; 3], T) {
        let r = lift::<T>(r);
        let u = sub3(&lift::<T>(s), &r);
        let v = sub3(&lift::<T>(t), &r);
        let ([ux, uy, uz], [vx, vy, vz]) = (&u, &v);
        let n = [det2(uy, uz, vy, vz), det2(uz, ux, vz, vx), det2(ux, uy, vx, vy)];
        let [nx, ny, nz] = &n;
        let [rx, ry, rz] = &r;
        let c = nx.clone() * rx.clone() + ny.clone() * ry.clone() + nz.clone() * rz.clone();
        (n, c)
    };
    let [p1, p2, p3] = planes;
    let [(n1, c1), (n2, c2), (n3, c3)] = [plane(p1), plane(p2), plane(p3)];
    // Row `n` with column `k` replaced by `c`.
    let with = |n: &[T; 3], c: &T, k: usize| -> [T; 3] { std::array::from_fn(|j| if j == k { c.clone() } else { coord(n, j) }) };
    let x = std::array::from_fn(|k| det3(&[with(&n1, &c1, k), with(&n2, &c2, k), with(&n3, &c3, k)]));
    Hom { x, w: Some(det3(&[n1, n2, n3])) }
}

/// Decides the sign of `W` exactly: filter first, integers if needed.
fn exact_w_sign(filter: &Hom<Interval>, exact: impl FnOnce() -> Hom<Dyadic>) -> Sign {
    w_sign(&filter.w).or_else(|| w_sign(&exact().w)).unwrap_or(Sign::Zero)
}

/// The intersection of the line through `q1, q2` with the plane through `r, s, t`.
#[derive(Debug, Clone)]
pub struct Lpi {
    points: [[f64; 3]; 5],
    filter: Hom<Interval>,
}

impl Lpi {
    /// Fails when the point is undefined: the line is parallel to the plane, `q1 = q2`, or
    /// `r, s, t` are collinear (decided exactly).
    pub fn new(q1: &Point3, q2: &Point3, r: &Point3, s: &Point3, t: &Point3) -> Result<Self> {
        let points = [q1.0, q2.0, r.0, s.0, t.0];
        let filter = lpi_hom::<Interval>(&points);
        if exact_w_sign(&filter, || lpi_hom::<Dyadic>(&points)).is_zero() {
            return Err(Error::Invalid("line-plane intersection is undefined (parallel or degenerate)".into()));
        }
        Ok(Self { points, filter })
    }
}

/// The intersection point of three planes, each through three points.
#[derive(Debug, Clone)]
pub struct Tpi {
    planes: [[[f64; 3]; 3]; 3],
    filter: Hom<Interval>,
}

impl Tpi {
    /// Fails when the planes do not meet in exactly one point, or a plane's points are collinear
    /// (decided exactly).
    pub fn new(planes: [[&Point3; 3]; 3]) -> Result<Self> {
        let planes = planes.map(|p| p.map(|q| q.0));
        let filter = tpi_hom::<Interval>(&planes);
        if exact_w_sign(&filter, || tpi_hom::<Dyadic>(&planes)).is_zero() {
            return Err(Error::Invalid("three-plane intersection is undefined (planes not independent)".into()));
        }
        Ok(Self { planes, filter })
    }
}

/// An explicit point or an implicit one, for the `generic_*` predicates.
#[derive(Debug, Clone)]
pub enum GenericPoint {
    Explicit(Point3),
    Lpi(Lpi),
    Tpi(Tpi),
}

impl From<Point3> for GenericPoint {
    fn from(p: Point3) -> Self {
        GenericPoint::Explicit(p)
    }
}

impl GenericPoint {
    fn explicit(&self) -> Option<&Point3> {
        match self {
            GenericPoint::Explicit(p) => Some(p),
            _ => None,
        }
    }

    fn filter_hom(&self) -> Hom<Interval> {
        match self {
            GenericPoint::Explicit(p) => Hom { x: lift(&p.0), w: None },
            GenericPoint::Lpi(l) => l.filter.clone(),
            GenericPoint::Tpi(t) => t.filter.clone(),
        }
    }

    fn exact_hom(&self) -> Hom<Dyadic> {
        match self {
            GenericPoint::Explicit(p) => Hom { x: lift(&p.0), w: None },
            GenericPoint::Lpi(l) => lpi_hom(&l.points),
            GenericPoint::Tpi(t) => tpi_hom(&t.planes),
        }
    }

    /// Approximate Cartesian coordinates, for output and display only: never base a decision on
    /// them.
    pub fn approx(&self) -> [f64; 3] {
        match self {
            GenericPoint::Explicit(p) => p.0,
            _ => {
                let h = self.filter_hom();
                let w = h.w.map_or(1.0, |w| w.mid());
                h.x.map(|x| x.mid() / w)
            }
        }
    }
}

/// Runs a formula through the interval filter, then exactly.
fn decide<const N: usize>(
    points: [&GenericPoint; N],
    filter: impl FnOnce([Hom<Interval>; N]) -> Option<Sign>,
    exact: impl FnOnce([Hom<Dyadic>; N]) -> Option<Sign>,
) -> Sign {
    filter(points.map(GenericPoint::filter_hom)).or_else(|| exact(points.map(GenericPoint::exact_hom))).unwrap_or(Sign::Zero)
}

/// Moves an explicit point (if any) to the last position, the pivot of the formulas, and
/// reports whether that took a transposition (which flips an orientation's sign).
fn pivot_last<const N: usize>(mut points: [&GenericPoint; N]) -> ([&GenericPoint; N], bool) {
    let last = N.saturating_sub(1);
    match points.iter().position(|p| p.explicit().is_some()) {
        Some(k) if k != last => {
            points.swap(k, last);
            (points, true)
        }
        _ => (points, false),
    }
}

/// `det[a - d; b - d; c - d] = det[R_a; R_b; R_c] / (W_a W_b W_c W_d³)` with
/// `R_i = W_d X_i - W_i X_d`.
fn orient3d_formula<T: Ring + Signed>([a, b, c, d]: [Hom<T>; 4]) -> Option<Sign> {
    let row = |p: &Hom<T>| -> [T; 3] { std::array::from_fn(|k| mulw(coord(&p.x, k), &d.w) - mulw(coord(&d.x, k), &p.w)) };
    let det = det3(&[row(&a), row(&b), row(&c)]).sign()?;
    Some(det * w_sign(&a.w)? * w_sign(&b.w)? * w_sign(&c.w)? * w_sign(&d.w)?)
}

/// [`orient3d`] on explicit or implicit points, same sign convention. Exact.
pub fn generic_orient3d(a: &GenericPoint, b: &GenericPoint, c: &GenericPoint, d: &GenericPoint) -> Sign {
    if let (Some(a), Some(b), Some(c), Some(d)) = (a.explicit(), b.explicit(), c.explicit(), d.explicit()) {
        return orient3d(a, b, c, d);
    }
    let (points, flipped) = pivot_last([a, b, c, d]);
    let sign = decide(points, orient3d_formula, orient3d_formula);
    if flipped { -sign } else { sign }
}

/// `det[a' - c'; b' - c']` of the projections: `det[R_a; R_b] / (W_a W_b W_c²)`.
fn orient2d_formula<T: Ring + Signed>([a, b, c]: [Hom<T>; 3], (u, v): (usize, usize)) -> Option<Sign> {
    let row = |p: &Hom<T>, k: usize| mulw(coord(&p.x, k), &c.w) - mulw(coord(&c.x, k), &p.w);
    let det = det2(&row(&a, u), &row(&a, v), &row(&b, u), &row(&b, v)).sign()?;
    Some(det * w_sign(&a.w)? * w_sign(&b.w)?)
}

/// [`orient2d`] of the projections of `a, b, c` onto the coordinate plane that drops `drop`,
/// seen from the positive side of that axis (`Z` → the `xy` plane, `X` → `yz`, `Y` → `zx`).
/// Exact. Attene's `orient2d3d` (§4.5) is this with the axis of the largest normal component.
pub fn generic_orient2d(a: &GenericPoint, b: &GenericPoint, c: &GenericPoint, drop: Axis) -> Sign {
    if let (Some(a), Some(b), Some(c)) = (a.explicit(), b.explicit(), c.explicit()) {
        return orient2d_projected(a, b, c, drop);
    }
    let (u, v) = drop.projected();
    let (points, flipped) = pivot_last([a, b, c]);
    let sign = decide(points, |h| orient2d_formula(h, (u, v)), |h| orient2d_formula(h, (u, v)));
    if flipped { -sign } else { sign }
}

/// [`orient2d`] of the projections of three explicit points onto the coordinate plane that drops
/// `drop`, seen from the positive side of that axis (the explicit case of [`generic_orient2d`]).
pub fn orient2d_projected(a: &Point3, b: &Point3, c: &Point3, drop: Axis) -> Sign {
    let (u, v) = drop.projected();
    let proj = |p: &Point3| Point2([coord(&p.0, u), coord(&p.0, v)]);
    orient2d(&proj(a), &proj(b), &proj(c))
}

/// `X_a / W_a - X_b / W_b = (W_b X_a - W_a X_b) / (W_a W_b)` along one axis.
fn compare_formula<T: Ring + Signed>([a, b]: [Hom<T>; 2], k: usize) -> Option<Sign> {
    let diff = mulw(coord(&a.x, k), &b.w) - mulw(coord(&b.x, k), &a.w);
    Some(diff.sign()? * w_sign(&a.w)? * w_sign(&b.w)?)
}

/// Compares the `axis` coordinates of two points exactly (Attene's `lessThan`).
pub fn generic_compare(a: &GenericPoint, b: &GenericPoint, axis: Axis) -> Ordering {
    let k = axis.index();
    if let (Some(a), Some(b)) = (a.explicit(), b.explicit()) {
        return coord(&a.0, k).partial_cmp(&coord(&b.0, k)).unwrap_or(Ordering::Equal);
    }
    match decide([a, b], |h| compare_formula(h, k), |h| compare_formula(h, k)) {
        Sign::Negative => Ordering::Less,
        Sign::Zero => Ordering::Equal,
        Sign::Positive => Ordering::Greater,
    }
}
