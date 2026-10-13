//! Error-free transformations (Shewchuk 1997, §2.3 and §2.5).
//!
//! Each function returns `(x, y)` with `x = fl(a ∘ b)` and `y` the exact roundoff, so that
//! `a ∘ b = x + y` exactly. They assume IEEE 754 binary64 with round-to-nearest-even, no
//! intermediate extended precision and no fused multiply-add contraction; that is what Rust's
//! `f64` guarantees on every target ModelCraft builds for (x86_64 with SSE2, aarch64, wasm32):
//! rustc never contracts `a * b + c` into an FMA and does not use x87. No overflow or underflow
//! may occur, which the coordinate domain of [`super::Point2`] / [`super::Point3`] guarantees.

/// `2^ceil(53 / 2) + 1`, the splitter for Dekker's product (§2.5, Theorem 17).
const SPLITTER: f64 = 134_217_729.0;

/// Fast-Two-Sum (Theorem 6). Requires `|a| >= |b|` (or `a == 0`).
#[inline(always)]
pub(crate) fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    (x, b - b_virtual)
}

/// Two-Sum (Theorem 7), no precondition on the magnitudes.
#[inline(always)]
pub(crate) fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    let a_virtual = x - b_virtual;
    let b_round = b - b_virtual;
    let a_round = a - a_virtual;
    (x, a_round + b_round)
}

/// The roundoff of `x = fl(a - b)`, given `x`.
#[inline(always)]
pub(crate) fn two_diff_tail(a: f64, b: f64, x: f64) -> f64 {
    let b_virtual = a - x;
    let a_virtual = x + b_virtual;
    let b_round = b_virtual - b;
    let a_round = a - a_virtual;
    a_round + b_round
}

/// Two-Diff: `a - b = x + y` exactly.
#[inline(always)]
pub(crate) fn two_diff(a: f64, b: f64) -> (f64, f64) {
    let x = a - b;
    (x, two_diff_tail(a, b, x))
}

/// Dekker's split (Theorem 17): `a = hi + lo` with each half fitting in 26 bits.
#[inline(always)]
pub(crate) fn split(a: f64) -> (f64, f64) {
    let c = SPLITTER * a;
    let a_big = c - a;
    let hi = c - a_big;
    (hi, a - hi)
}

/// Two-Product with `b` already split (Theorem 18).
#[inline(always)]
pub(crate) fn two_product_presplit(a: f64, b: f64, b_hi: f64, b_lo: f64) -> (f64, f64) {
    let x = a * b;
    let (a_hi, a_lo) = split(a);
    let err1 = x - a_hi * b_hi;
    let err2 = err1 - a_lo * b_hi;
    let err3 = err2 - a_hi * b_lo;
    (x, a_lo * b_lo - err3)
}

/// Two-Product (Theorem 18): `a * b = x + y` exactly.
#[inline(always)]
pub(crate) fn two_product(a: f64, b: f64) -> (f64, f64) {
    let (b_hi, b_lo) = split(b);
    two_product_presplit(a, b, b_hi, b_lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_are_exact_on_classic_cases() {
        let (x, y) = two_sum(1.0, 1e-17);
        assert_eq!((x, y), (1.0, 1e-17));
        // 2^53 + 1 is a tie that rounds to the even 2^53.
        let (x, y) = fast_two_sum(2f64.powi(53), 1.0);
        assert_eq!((x, y), (2f64.powi(53), 1.0));
        let (x, y) = two_diff(1.0, 1e-17);
        assert_eq!((x, y), (1.0, -1e-17));
        // (1 + 2^-30)^2 = 1 + 2^-29 + 2^-60: the last term is the roundoff.
        let a = 1.0 + 2f64.powi(-30);
        let (x, y) = two_product(a, a);
        assert_eq!((x, y), (1.0 + 2f64.powi(-29), 2f64.powi(-60)));
        let (hi, lo) = split(a);
        assert_eq!(hi + lo, a);
    }
}
