//! Small 3-vector helpers on `[f64; 3]` (floating point: measures and display, never meshing
//! decisions, which use [`crate::predicates`]).

pub(crate) type P = [f64; 3];

pub(crate) fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(crate) fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn det(a: P, b: P, c: P) -> f64 {
    dot(a, cross(b, c))
}

/// The angle between two vectors in degrees (0 when either is zero).
pub(crate) fn angle_deg(a: P, b: P) -> f64 {
    atan2(norm(cross(a, b)), dot(a, b)).to_degrees()
}

/// `atan(t)` for `0 ≤ t ≤ 1` from `+ − × ÷ √` only: two half-angle reductions
/// (`atan t = 2 atan(t / (1 + √(1 + t²)))`) bring `t` below tan(π/16) ≈ 0.199, where the Taylor
/// series converges to double precision in 20 terms.
fn atan_unit(t: f64) -> f64 {
    let mut t = t;
    for _ in 0..2 {
        t /= 1.0 + (1.0 + t * t).sqrt();
    }
    let (t2, mut term, mut sum) = (t * t, t, 0.0);
    for k in 0..20 {
        let s = term / (2 * k + 1) as f64;
        sum += if k % 2 == 0 { s } else { -s };
        term *= t2;
    }
    4.0 * sum
}

/// The angle of `(x, y)` in radians, in `(−π, π]`, like [`f64::atan2`] but the same bit for bit
/// on every platform: the platform maths libraries' `atan2` differ in the last bits (macOS and
/// glibc), and a mesh or report must not (AGENTS.md, Determinism). 0 for `(0, 0)` and for
/// non-finite input.
pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    if !(x.is_finite() && y.is_finite()) || (x == 0.0 && y == 0.0) {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let a = if ay <= ax { atan_unit(ay / ax) } else { FRAC_PI_2 - atan_unit(ax / ay) };
    let a = if x < 0.0 { PI - a } else { a };
    if y < 0.0 { -a } else { a }
}

/// The cube root, from `+ − × ÷` only (Newton's iteration from an exact power-of-two estimate),
/// the same on every platform (see [`atan2`]).
pub(crate) fn cbrt(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let a = x.abs();
    // 2^(e/3) for a's binary exponent e: within a factor 2 of the root.
    let e = ((a.to_bits() >> 52) & 0x7ff) as i64 - 1023;
    let mut r = f64::from_bits((((e / 3) + 1023).clamp(1, 2046) as u64) << 52);
    for _ in 0..60 {
        let next = r - (r * r * r - a) / (3.0 * r * r);
        if next == r || !next.is_finite() {
            break;
        }
        r = next;
    }
    if x < 0.0 { -r } else { r }
}
pub(crate) fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub(crate) fn dist(a: P, b: P) -> f64 {
    norm(sub(a, b))
}
pub(crate) fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
pub(crate) fn mid(a: P, b: P) -> P {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.5 * (a[2] + b[2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portable atan2 and cbrt agree with the platform's to a few ulps.
    #[test]
    fn portable_functions_match_the_platform() {
        let mut worst: f64 = 0.0;
        for i in 0..2000 {
            let a = -std::f64::consts::PI + i as f64 * (2.0 * std::f64::consts::PI / 1999.0);
            for r in [1e-300, 1e-3, 1.0, 7.5, 1e200] {
                let (y, x) = (r * a.sin(), r * a.cos());
                worst = worst.max((atan2(y, x) - y.atan2(x)).abs());
            }
        }
        assert!(worst < 4e-15, "{worst}");
        assert_eq!(atan2(0.0, 0.0), 0.0);
        assert_eq!(atan2(1.0, 0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(atan2(0.0, -1.0), std::f64::consts::PI);
        for x in [1e-300, 2.5e-7, 0.3, 1.0, 8.0, 27.0, 1234.5, 1e300, -64.0] {
            let rel: f64 = (cbrt(x) - x.cbrt()).abs() / x.cbrt().abs();
            assert!(rel < 4e-16, "{x}: {} vs {}", cbrt(x), x.cbrt());
        }
        assert_eq!(cbrt(0.0), 0.0);
    }
}
