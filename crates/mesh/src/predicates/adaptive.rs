//! Shewchuk's adaptive-precision `orient2d`, `orient3d`, `incircle` and `insphere`
//! (Shewchuk 1997, §4 and §5), implemented from the paper.
//!
//! Each predicate evaluates its determinant in plain doubles and accepts the sign when the
//! magnitude exceeds the paper's certified error bound (stage A, the filter). Otherwise it
//! escalates: stage B computes the determinant of the rounded coordinate differences exactly as
//! an expansion and accepts it under a tighter bound, or outright when the differences were exact
//! (zero tails, the common case for grid-like CAD input); stage C (`orient2d`, `orient3d`) adds
//! the first-order tail correction; the last stage evaluates the determinant exactly from the
//! input coordinates with expansion arithmetic. Shewchuk's further hand-scheduled stages between
//! C and exact are not reproduced: they only make rare inputs faster.
//!
//! The error bounds assume IEEE binary64 round-to-nearest-even arithmetic without overflow or
//! underflow (§5); the coordinate domain of [`Point2`] / [`Point3`] rules both out.

use super::eft::two_diff_tail;
use super::expansion::{Exp, Expansion, cross_diff};
use super::ring::{det2, det3, sq2, sq3};
use super::{Point2, Point3, Sign};

/// `ε = 2^-53`, half a unit in the last place of 1 (the machine epsilon of §2.1).
const EPS: f64 = f64::EPSILON / 2.0;
const RESULT_ERR: f64 = (3.0 + 8.0 * EPS) * EPS;
const CCW_A: f64 = (3.0 + 16.0 * EPS) * EPS;
const CCW_B: f64 = (2.0 + 12.0 * EPS) * EPS;
const CCW_C: f64 = (9.0 + 64.0 * EPS) * EPS * EPS;
const O3D_A: f64 = (7.0 + 56.0 * EPS) * EPS;
const O3D_B: f64 = (3.0 + 28.0 * EPS) * EPS;
const O3D_C: f64 = (26.0 + 288.0 * EPS) * EPS * EPS;
const ICC_A: f64 = (10.0 + 96.0 * EPS) * EPS;
const ICC_B: f64 = (4.0 + 48.0 * EPS) * EPS;
const ISP_A: f64 = (16.0 + 224.0 * EPS) * EPS;
const ISP_B: f64 = (5.0 + 72.0 * EPS) * EPS;

#[inline(always)]
fn certain(det: f64, errbound: f64) -> bool {
    det > errbound || -det > errbound
}

// ---------------------------------------------------------------------------------------------
// orient2d
// ---------------------------------------------------------------------------------------------

/// Stage A of [`orient2d`]: the sign when the double-precision filter certifies it.
#[inline]
pub fn orient2d_filter(a: &Point2, b: &Point2, c: &Point2) -> Option<Sign> {
    orient2d_a(a, b, c).ok()
}

/// `Ok(sign)` when stage A decides, else `Err(detsum)` for the adaptive stages.
#[inline(always)]
fn orient2d_a(a: &Point2, b: &Point2, c: &Point2) -> Result<Sign, f64> {
    let ([ax, ay], [bx, by], [cx, cy]) = (a.0, b.0, c.0);
    let det_left = (ax - cx) * (by - cy);
    let det_right = (ay - cy) * (bx - cx);
    let det = det_left - det_right;
    // When the two products have opposite signs (or one is zero) the subtraction cannot cancel
    // and the rounded result has the exact sign.
    let det_sum = if det_left > 0.0 {
        if det_right <= 0.0 {
            return Ok(Sign::of(det));
        }
        det_left + det_right
    } else if det_left < 0.0 {
        if det_right >= 0.0 {
            return Ok(Sign::of(det));
        }
        -det_left - det_right
    } else {
        return Ok(Sign::of(det));
    };
    if certain(det, CCW_A * det_sum) { Ok(Sign::of(det)) } else { Err(det_sum) }
}

/// The orientation of `a, b, c`: [`Sign::Positive`] when they turn counterclockwise,
/// [`Sign::Negative`] when clockwise, [`Sign::Zero`] when collinear. Exact. It is the sign of
/// `det[a - c; b - c]`.
#[inline]
pub fn orient2d(a: &Point2, b: &Point2, c: &Point2) -> Sign {
    match orient2d_a(a, b, c) {
        Ok(sign) => sign,
        Err(det_sum) => orient2d_adapt(a, b, c, det_sum),
    }
}

#[cold]
fn orient2d_adapt(a: &Point2, b: &Point2, c: &Point2, det_sum: f64) -> Sign {
    let ([ax, ay], [bx, by], [cx, cy]) = (a.0, b.0, c.0);
    let (acx, bcx, acy, bcy) = (ax - cx, bx - cx, ay - cy, by - cy);

    // Stage B: the exact determinant of the rounded differences.
    let b_exp = cross_diff(acx, bcy, acy, bcx);
    let det = b_exp.estimate();
    if certain(det, CCW_B * det_sum) {
        return Sign::of(det);
    }
    let acx_t = two_diff_tail(ax, cx, acx);
    let bcx_t = two_diff_tail(bx, cx, bcx);
    let acy_t = two_diff_tail(ay, cy, acy);
    let bcy_t = two_diff_tail(by, cy, bcy);
    if acx_t == 0.0 && acy_t == 0.0 && bcx_t == 0.0 && bcy_t == 0.0 {
        return b_exp.sign();
    }

    // Stage C: first-order correction for the tails.
    let errbound = CCW_C * det_sum + RESULT_ERR * det.abs();
    let det = det + ((acx * bcy_t + bcy * acx_t) - (acy * bcx_t + bcx * acy_t));
    if certain(det, errbound) {
        return Sign::of(det);
    }

    // Stage D: the remaining terms of (acx + acx_t)(bcy + bcy_t) - (acy + acy_t)(bcx + bcx_t).
    let c1 = Exp::<8>::sum(&b_exp, &cross_diff(acx_t, bcy, acy_t, bcx));
    let c2 = Exp::<12>::sum(&c1, &cross_diff(acx, bcy_t, acy, bcx_t));
    Exp::<16>::sum(&c2, &cross_diff(acx_t, bcy_t, acy_t, bcx_t)).sign()
}

// ---------------------------------------------------------------------------------------------
// orient3d
// ---------------------------------------------------------------------------------------------

/// Stage A of [`orient3d`].
#[inline]
pub fn orient3d_filter(a: &Point3, b: &Point3, c: &Point3, d: &Point3) -> Option<Sign> {
    orient3d_a(a, b, c, d).ok()
}

#[inline(always)]
fn orient3d_a(a: &Point3, b: &Point3, c: &Point3, d: &Point3) -> Result<Sign, f64> {
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz], [dx, dy, dz]) = (a.0, b.0, c.0, d.0);
    let (adx, bdx, cdx) = (ax - dx, bx - dx, cx - dx);
    let (ady, bdy, cdy) = (ay - dy, by - dy, cy - dy);
    let (adz, bdz, cdz) = (az - dz, bz - dz, cz - dz);
    let (bdxcdy, cdxbdy) = (bdx * cdy, cdx * bdy);
    let (cdxady, adxcdy) = (cdx * ady, adx * cdy);
    let (adxbdy, bdxady) = (adx * bdy, bdx * ady);
    let det = adz * (bdxcdy - cdxbdy) + bdz * (cdxady - adxcdy) + cdz * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * adz.abs() + (cdxady.abs() + adxcdy.abs()) * bdz.abs() + (adxbdy.abs() + bdxady.abs()) * cdz.abs();
    if certain(det, O3D_A * permanent) { Ok(Sign::of(det)) } else { Err(permanent) }
}

/// The orientation of `d` relative to the plane through `a, b, c`: [`Sign::Positive`] when `d`
/// lies below the plane, "above" being the side from which `a, b, c` appear counterclockwise;
/// [`Sign::Zero`] when the four points are coplanar. Exact. It is the sign of
/// `det[a - d; b - d; c - d] = -det[b - a; c - a; d - a]` (Shewchuk's convention), so it is
/// positive when the tetrahedron `a, b, c, d` is negatively oriented in the right-handed sense.
#[inline]
pub fn orient3d(a: &Point3, b: &Point3, c: &Point3, d: &Point3) -> Sign {
    match orient3d_a(a, b, c, d) {
        Ok(sign) => sign,
        Err(permanent) => orient3d_adapt(a, b, c, d, permanent),
    }
}

#[cold]
fn orient3d_adapt(a: &Point3, b: &Point3, c: &Point3, d: &Point3, permanent: f64) -> Sign {
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz], [dx, dy, dz]) = (a.0, b.0, c.0, d.0);
    let (adx, bdx, cdx) = (ax - dx, bx - dx, cx - dx);
    let (ady, bdy, cdy) = (ay - dy, by - dy, cy - dy);
    let (adz, bdz, cdz) = (az - dz, bz - dz, cz - dz);

    // Stage B.
    let bc = cross_diff(bdx, cdy, cdx, bdy);
    let ca = cross_diff(cdx, ady, adx, cdy);
    let ab = cross_diff(adx, bdy, bdx, ady);
    let a_det = Exp::<8>::scale(&bc, adz);
    let b_det = Exp::<8>::scale(&ca, bdz);
    let c_det = Exp::<8>::scale(&ab, cdz);
    let fin = Exp::<24>::sum(&Exp::<16>::sum(&a_det, &b_det), &c_det);
    let det = fin.estimate();
    if certain(det, O3D_B * permanent) {
        return Sign::of(det);
    }
    let (adx_t, bdx_t, cdx_t) = (two_diff_tail(ax, dx, adx), two_diff_tail(bx, dx, bdx), two_diff_tail(cx, dx, cdx));
    let (ady_t, bdy_t, cdy_t) = (two_diff_tail(ay, dy, ady), two_diff_tail(by, dy, bdy), two_diff_tail(cy, dy, cdy));
    let (adz_t, bdz_t, cdz_t) = (two_diff_tail(az, dz, adz), two_diff_tail(bz, dz, bdz), two_diff_tail(cz, dz, cdz));
    let tails = [adx_t, bdx_t, cdx_t, ady_t, bdy_t, cdy_t, adz_t, bdz_t, cdz_t];
    if tails.iter().all(|&t| t == 0.0) {
        return fin.sign();
    }

    // Stage C.
    let errbound = O3D_C * permanent + RESULT_ERR * det.abs();
    let det = det
        + (adz * ((bdx * cdy_t + cdy * bdx_t) - (bdy * cdx_t + cdx * bdy_t)) + adz_t * (bdx * cdy - bdy * cdx))
        + (bdz * ((cdx * ady_t + ady * cdx_t) - (cdy * adx_t + adx * cdy_t)) + bdz_t * (cdx * ady - cdy * adx))
        + (cdz * ((adx * bdy_t + bdy * adx_t) - (ady * bdx_t + bdx * ady_t)) + cdz_t * (adx * bdy - ady * bdx));
    if certain(det, errbound) {
        return Sign::of(det);
    }
    orient3d_exact(a, b, c, d)
}

/// `det[a - d; b - d; c - d]` exactly, from the input coordinates.
fn orient3d_exact(a: &Point3, b: &Point3, c: &Point3, d: &Point3) -> Sign {
    det3(&[diffs3(a, d), diffs3(b, d), diffs3(c, d)]).sign()
}

/// The exact coordinate differences `p - o` as expansions.
fn diffs3(p: &Point3, o: &Point3) -> [Expansion; 3] {
    let ([px, py, pz], [ox, oy, oz]) = (p.0, o.0);
    [Expansion::diff(px, ox), Expansion::diff(py, oy), Expansion::diff(pz, oz)]
}

fn diffs2(p: &Point2, o: &Point2) -> [Expansion; 2] {
    let ([px, py], [ox, oy]) = (p.0, o.0);
    [Expansion::diff(px, ox), Expansion::diff(py, oy)]
}

// ---------------------------------------------------------------------------------------------
// incircle
// ---------------------------------------------------------------------------------------------

/// Stage A of [`incircle`].
#[inline]
pub fn incircle_filter(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Option<Sign> {
    incircle_a(a, b, c, d).ok()
}

#[inline(always)]
fn incircle_a(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Result<Sign, f64> {
    let ([ax, ay], [bx, by], [cx, cy], [dx, dy]) = (a.0, b.0, c.0, d.0);
    let (adx, bdx, cdx) = (ax - dx, bx - dx, cx - dx);
    let (ady, bdy, cdy) = (ay - dy, by - dy, cy - dy);
    let (bdxcdy, cdxbdy) = (bdx * cdy, cdx * bdy);
    let a_lift = adx * adx + ady * ady;
    let (cdxady, adxcdy) = (cdx * ady, adx * cdy);
    let b_lift = bdx * bdx + bdy * bdy;
    let (adxbdy, bdxady) = (adx * bdy, bdx * ady);
    let c_lift = cdx * cdx + cdy * cdy;
    let det = a_lift * (bdxcdy - cdxbdy) + b_lift * (cdxady - adxcdy) + c_lift * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * a_lift + (cdxady.abs() + adxcdy.abs()) * b_lift + (adxbdy.abs() + bdxady.abs()) * c_lift;
    if certain(det, ICC_A * permanent) { Ok(Sign::of(det)) } else { Err(permanent) }
}

/// Whether `d` lies inside the circle through `a, b, c`: [`Sign::Positive`] inside,
/// [`Sign::Negative`] outside, [`Sign::Zero`] on it, for `a, b, c` counterclockwise (the sign is
/// reversed when they are clockwise). Exact. It is the sign of
/// `det[a - d, |a - d|²; b - d, |b - d|²; c - d, |c - d|²]`.
#[inline]
pub fn incircle(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Sign {
    match incircle_a(a, b, c, d) {
        Ok(sign) => sign,
        Err(permanent) => incircle_adapt(a, b, c, d, permanent),
    }
}

#[cold]
fn incircle_adapt(a: &Point2, b: &Point2, c: &Point2, d: &Point2, permanent: f64) -> Sign {
    let ([ax, ay], [bx, by], [cx, cy], [dx, dy]) = (a.0, b.0, c.0, d.0);
    let (adx, bdx, cdx) = (ax - dx, bx - dx, cx - dx);
    let (ady, bdy, cdy) = (ay - dy, by - dy, cy - dy);

    // Stage B.
    let lifted = |minor: &Exp<4>, x: f64, y: f64| -> Exp<32> {
        Exp::sum(&Exp::<16>::scale(&Exp::<8>::scale(minor, x), x), &Exp::<16>::scale(&Exp::<8>::scale(minor, y), y))
    };
    let a_det = lifted(&cross_diff(bdx, cdy, cdx, bdy), adx, ady);
    let b_det = lifted(&cross_diff(cdx, ady, adx, cdy), bdx, bdy);
    let c_det = lifted(&cross_diff(adx, bdy, bdx, ady), cdx, cdy);
    let fin = Exp::<96>::sum(&Exp::<64>::sum(&a_det, &b_det), &c_det);
    let det = fin.estimate();
    if certain(det, ICC_B * permanent) {
        return Sign::of(det);
    }
    let tails = [
        two_diff_tail(ax, dx, adx),
        two_diff_tail(bx, dx, bdx),
        two_diff_tail(cx, dx, cdx),
        two_diff_tail(ay, dy, ady),
        two_diff_tail(by, dy, bdy),
        two_diff_tail(cy, dy, cdy),
    ];
    if tails.iter().all(|&t| t == 0.0) {
        return fin.sign();
    }
    incircle_exact(a, b, c, d)
}

fn incircle_exact(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Sign {
    let row = |p: &Point2| {
        let [x, y] = diffs2(p, d);
        let lift = sq2(&x, &y);
        [x, y, lift]
    };
    det3(&[row(a), row(b), row(c)]).sign()
}

// ---------------------------------------------------------------------------------------------
// insphere
// ---------------------------------------------------------------------------------------------

/// Stage A of [`insphere`].
#[inline]
pub fn insphere_filter(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Option<Sign> {
    insphere_a(a, b, c, d, e).ok()
}

#[inline(always)]
fn insphere_a(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Result<Sign, f64> {
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz], [dx, dy, dz], [ex, ey, ez]) = (a.0, b.0, c.0, d.0, e.0);
    let (aex, bex, cex, dex) = (ax - ex, bx - ex, cx - ex, dx - ex);
    let (aey, bey, cey, dey) = (ay - ey, by - ey, cy - ey, dy - ey);
    let (aez, bez, cez, dez) = (az - ez, bz - ez, cz - ez, dz - ez);

    let (aexbey, bexaey) = (aex * bey, bex * aey);
    let ab = aexbey - bexaey;
    let (bexcey, cexbey) = (bex * cey, cex * bey);
    let bc = bexcey - cexbey;
    let (cexdey, dexcey) = (cex * dey, dex * cey);
    let cd = cexdey - dexcey;
    let (dexaey, aexdey) = (dex * aey, aex * dey);
    let da = dexaey - aexdey;
    let (aexcey, cexaey) = (aex * cey, cex * aey);
    let ac = aexcey - cexaey;
    let (bexdey, dexbey) = (bex * dey, dex * bey);
    let bd = bexdey - dexbey;

    let abc = aez * bc - bez * ac + cez * ab;
    let bcd = bez * cd - cez * bd + dez * bc;
    let cda = cez * da + dez * ac + aez * cd;
    let dab = dez * ab + aez * bd + bez * da;

    let a_lift = aex * aex + aey * aey + aez * aez;
    let b_lift = bex * bex + bey * bey + bez * bez;
    let c_lift = cex * cex + cey * cey + cez * cez;
    let d_lift = dex * dex + dey * dey + dez * dez;

    let det = (d_lift * abc - c_lift * dab) + (b_lift * cda - a_lift * bcd);

    let (aez_p, bez_p, cez_p, dez_p) = (aez.abs(), bez.abs(), cez.abs(), dez.abs());
    let (ab_p, bc_p, cd_p) = (aexbey.abs() + bexaey.abs(), bexcey.abs() + cexbey.abs(), cexdey.abs() + dexcey.abs());
    let (da_p, ac_p, bd_p) = (dexaey.abs() + aexdey.abs(), aexcey.abs() + cexaey.abs(), bexdey.abs() + dexbey.abs());
    let permanent = (cd_p * bez_p + bd_p * cez_p + bc_p * dez_p) * a_lift
        + (da_p * cez_p + ac_p * dez_p + cd_p * aez_p) * b_lift
        + (ab_p * dez_p + bd_p * aez_p + da_p * bez_p) * c_lift
        + (bc_p * aez_p + ac_p * bez_p + ab_p * cez_p) * d_lift;
    if certain(det, ISP_A * permanent) { Ok(Sign::of(det)) } else { Err(permanent) }
}

/// Whether `e` lies inside the sphere through `a, b, c, d`: [`Sign::Positive`] inside,
/// [`Sign::Negative`] outside, [`Sign::Zero`] on it, for `orient3d(a, b, c, d) > 0` (the sign is
/// reversed otherwise). Exact. It is the sign of the 4×4 determinant with rows
/// `[p - e, |p - e|²]` for `p = a, b, c, d`.
#[inline]
pub fn insphere(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Sign {
    match insphere_a(a, b, c, d, e) {
        Ok(sign) => sign,
        Err(permanent) => insphere_adapt(a, b, c, d, e, permanent),
    }
}

#[cold]
fn insphere_adapt(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3, permanent: f64) -> Sign {
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz], [dx, dy, dz], [ex, ey, ez]) = (a.0, b.0, c.0, d.0, e.0);
    let (aex, bex, cex, dex) = (ax - ex, bx - ex, cx - ex, dx - ex);
    let (aey, bey, cey, dey) = (ay - ey, by - ey, cy - ey, dy - ey);
    let (aez, bez, cez, dez) = (az - ez, bz - ez, cz - ez, dz - ez);

    // Stage B: the 2×2 minors of the x, y columns, the 3×3 minors, then the lifted terms.
    let ab = cross_diff(aex, bey, bex, aey);
    let bc = cross_diff(bex, cey, cex, bey);
    let cd = cross_diff(cex, dey, dex, cey);
    let da = cross_diff(dex, aey, aex, dey);
    let ac = cross_diff(aex, cey, cex, aey);
    let bd = cross_diff(bex, dey, dex, bey);
    let minor3 = |m1: &Exp<4>, z1: f64, m2: &Exp<4>, z2: f64, m3: &Exp<4>, z3: f64| -> Exp<24> {
        Exp::sum(&Exp::<16>::sum(&Exp::<8>::scale(m1, z1), &Exp::<8>::scale(m2, z2)), &Exp::<8>::scale(m3, z3))
    };
    let abc = minor3(&bc, aez, &ac, -bez, &ab, cez);
    let bcd = minor3(&cd, bez, &bd, -cez, &bc, dez);
    let cda = minor3(&da, cez, &ac, dez, &cd, aez);
    let dab = minor3(&ab, dez, &bd, aez, &da, bez);
    let lifted = |m: &Exp<24>, x: f64, y: f64, z: f64| -> Exp<288> {
        let xx = Exp::<96>::scale(&Exp::<48>::scale(m, x), x);
        let yy = Exp::<96>::scale(&Exp::<48>::scale(m, y), y);
        let zz = Exp::<96>::scale(&Exp::<48>::scale(m, z), z);
        Exp::sum(&Exp::<192>::sum(&xx, &yy), &zz)
    };
    let d_term = lifted(&abc, dex, dey, dez);
    let c_term = lifted(&dab, cex, cey, cez).negated();
    let b_term = lifted(&cda, bex, bey, bez);
    let a_term = lifted(&bcd, aex, aey, aez).negated();
    let fin = Exp::<1152>::sum(&Exp::<576>::sum(&d_term, &c_term), &Exp::<576>::sum(&b_term, &a_term));
    let det = fin.estimate();
    if certain(det, ISP_B * permanent) {
        return Sign::of(det);
    }
    let tails = [
        two_diff_tail(ax, ex, aex),
        two_diff_tail(bx, ex, bex),
        two_diff_tail(cx, ex, cex),
        two_diff_tail(dx, ex, dex),
        two_diff_tail(ay, ey, aey),
        two_diff_tail(by, ey, bey),
        two_diff_tail(cy, ey, cey),
        two_diff_tail(dy, ey, dey),
        two_diff_tail(az, ez, aez),
        two_diff_tail(bz, ez, bez),
        two_diff_tail(cz, ez, cez),
        two_diff_tail(dz, ez, dez),
    ];
    if tails.iter().all(|&t| t == 0.0) {
        return fin.sign();
    }
    insphere_exact(a, b, c, d, e)
}

/// The same expansion as stage B (shared 2×2 minors, then the 3×3 minors, then the lifts), on
/// the exact differences.
fn insphere_exact(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Sign {
    let [ax, ay, az] = diffs3(a, e);
    let [bx, by, bz] = diffs3(b, e);
    let [cx, cy, cz] = diffs3(c, e);
    let [dx, dy, dz] = diffs3(d, e);
    let ab = det2(&ax, &ay, &bx, &by);
    let bc = det2(&bx, &by, &cx, &cy);
    let cd = det2(&cx, &cy, &dx, &dy);
    let da = det2(&dx, &dy, &ax, &ay);
    let ac = det2(&ax, &ay, &cx, &cy);
    let bd = det2(&bx, &by, &dx, &dy);
    let abc = az.clone() * bc.clone() - bz.clone() * ac.clone() + cz.clone() * ab.clone();
    let bcd = bz.clone() * cd.clone() - cz.clone() * bd.clone() + dz.clone() * bc;
    let cda = cz.clone() * da.clone() + dz.clone() * ac + az.clone() * cd;
    let dab = dz.clone() * ab + az.clone() * bd + bz.clone() * da;
    let lift = sq3(&dx, &dy, &dz) * abc - sq3(&cx, &cy, &cz) * dab + sq3(&bx, &by, &bz) * cda - sq3(&ax, &ay, &az) * bcd;
    lift.sign()
}

/// The exact stages alone, for tests that exercise them directly.
#[cfg(test)]
pub(crate) mod exact {
    use super::*;

    pub(crate) fn orient2d(a: &Point2, b: &Point2, c: &Point2) -> Sign {
        let [ax, ay] = diffs2(a, c);
        let [bx, by] = diffs2(b, c);
        det2(&ax, &ay, &bx, &by).sign()
    }
    pub(crate) fn orient3d(a: &Point3, b: &Point3, c: &Point3, d: &Point3) -> Sign {
        orient3d_exact(a, b, c, d)
    }
    pub(crate) fn incircle(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Sign {
        incircle_exact(a, b, c, d)
    }
    pub(crate) fn insphere(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Sign {
        insphere_exact(a, b, c, d, e)
    }
}
