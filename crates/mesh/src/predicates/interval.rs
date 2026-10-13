//! Interval arithmetic, the dynamic filter of the indirect predicates (Attene 2020, §5.2;
//! Brönnimann, Burnikel & Pion 1998).
//!
//! Attene switches the FPU to directed rounding; Rust has no portable control of the rounding
//! mode (and wasm none at all), so each bound is computed with the default round-to-nearest and
//! then moved outward by one unit in the last place (`next_down` / `next_up`). A round-to-nearest
//! result is within half an ulp of the exact value, so the widened interval always contains it,
//! subnormals included. Overflow gives infinite bounds and `inf - inf` gives NaN; both make the
//! sign uncertain, which sends the predicate on to the exact stage.

use std::ops::{Add, Mul, Sub};

use super::Sign;
use super::ring::{Ring, Signed};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Interval {
    lo: f64,
    hi: f64,
}

impl Interval {
    const NAN: Interval = Interval { lo: f64::NAN, hi: f64::NAN };

    #[inline(always)]
    fn widened(lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() { Self::NAN } else { Self { lo: lo.next_down(), hi: hi.next_up() } }
    }

    /// The midpoint, an approximation of the value.
    pub(crate) fn mid(&self) -> f64 {
        0.5 * self.lo + 0.5 * self.hi
    }
}

impl Add for Interval {
    type Output = Self;

    #[inline(always)]
    fn add(self, rhs: Self) -> Self {
        Self::widened(self.lo + rhs.lo, self.hi + rhs.hi)
    }
}

impl Sub for Interval {
    type Output = Self;

    #[inline(always)]
    fn sub(self, rhs: Self) -> Self {
        Self::widened(self.lo - rhs.hi, self.hi - rhs.lo)
    }
}

impl Mul for Interval {
    type Output = Self;

    #[inline(always)]
    fn mul(self, rhs: Self) -> Self {
        let p = [self.lo * rhs.lo, self.lo * rhs.hi, self.hi * rhs.lo, self.hi * rhs.hi];
        // `f64::min`/`max` skip NaN (from `0 * inf`), which would drop a bound: check first.
        if p.iter().any(|x| x.is_nan()) {
            return Self::NAN;
        }
        let lo = p.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = p.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::widened(lo, hi)
    }
}

impl Ring for Interval {
    #[inline(always)]
    fn from_f64(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
}

impl Signed for Interval {
    #[inline(always)]
    fn sign(&self) -> Option<Sign> {
        if self.lo > 0.0 {
            Some(Sign::Positive)
        } else if self.hi < 0.0 {
            Some(Sign::Negative)
        } else if self.lo == 0.0 && self.hi == 0.0 {
            Some(Sign::Zero)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_enclose_and_flag_uncertainty() {
        let third = Interval::from_f64(1.0) * Interval::from_f64(1.0 / 3.0);
        let x = third + third + third - Interval::from_f64(1.0);
        assert!(x.lo <= 0.0 && x.hi >= 0.0);
        assert_eq!(x.sign(), None);
        let big = Interval::from_f64(f64::MAX) * Interval::from_f64(2.0);
        assert_eq!((big - big).sign(), None);
        assert_eq!((Interval::from_f64(2.0) * Interval::from_f64(3.0)).sign(), Some(Sign::Positive));
    }
}
