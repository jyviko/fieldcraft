//! Floating-point expansions (Shewchuk 1997, §2.2–§2.6).
//!
//! An expansion is a sum of doubles, stored in order of increasing magnitude, whose components
//! are nonoverlapping (here always nonadjacent or strongly nonoverlapping, as the theorems that
//! the algorithms below rely on require). Zero components are eliminated, so a zero expansion is
//! the single component `0.0` and the sign of any expansion is the sign of its last component.
//!
//! The two algorithms (Fast-Expansion-Sum with zero elimination, Theorem 13; Scale-Expansion with
//! zero elimination, Theorem 19) are written once against a sink. [`Exp`] is a fixed-capacity
//! stack expansion for the adaptive stages, whose capacities are checked at compile time;
//! [`Expansion`] is a heap expansion with exact `+`, `-` and `*` for the final exact stage.

use std::ops::{Add, Mul, Neg, Sub};

use super::Sign;
use super::eft::{fast_two_sum, split, two_diff, two_product, two_product_presplit, two_sum};
use super::ring::{Ring, Signed};

/// Merges two expansions into one stream of components of nondecreasing magnitude.
struct Merge<'a> {
    e: &'a [f64],
    f: &'a [f64],
}

impl Iterator for Merge<'_> {
    type Item = f64;

    #[inline(always)]
    fn next(&mut self) -> Option<f64> {
        match (self.e.split_first(), self.f.split_first()) {
            (Some((&a, e_rest)), Some((&b, f_rest))) => {
                // `(b > a) == (b > -a)` is true when |a| < |b| (Shewchuk's comparison).
                if (b > a) == (b > -a) {
                    self.e = e_rest;
                    Some(a)
                } else {
                    self.f = f_rest;
                    Some(b)
                }
            }
            (Some((&a, rest)), None) => {
                self.e = rest;
                Some(a)
            }
            (None, Some((&b, rest))) => {
                self.f = rest;
                Some(b)
            }
            (None, None) => None,
        }
    }
}

/// Fast-Expansion-Sum with zero elimination: emits `e + f`, at most `e.len() + f.len()`
/// components.
#[inline]
fn sum_into(e: &[f64], f: &[f64], mut emit: impl FnMut(f64)) {
    let mut merged = Merge { e, f };
    let Some(mut q) = merged.next() else {
        emit(0.0);
        return;
    };
    let mut emitted = false;
    if let Some(x) = merged.next() {
        let (q_new, h) = fast_two_sum(x, q);
        q = q_new;
        if h != 0.0 {
            emit(h);
            emitted = true;
        }
        for x in merged {
            let (q_new, h) = two_sum(q, x);
            q = q_new;
            if h != 0.0 {
                emit(h);
                emitted = true;
            }
        }
    }
    if q != 0.0 || !emitted {
        emit(q);
    }
}

/// Scale-Expansion with zero elimination: emits `e * b`, at most `2 * e.len()` components.
#[inline]
fn scale_into(e: &[f64], b: f64, mut emit: impl FnMut(f64)) {
    let (b_hi, b_lo) = split(b);
    let mut components = e.iter().copied();
    let Some(first) = components.next() else {
        emit(0.0);
        return;
    };
    let mut emitted = false;
    let (mut q, h) = two_product_presplit(first, b, b_hi, b_lo);
    if h != 0.0 {
        emit(h);
        emitted = true;
    }
    for component in components {
        let (p1, p0) = two_product_presplit(component, b, b_hi, b_lo);
        let (s, h) = two_sum(q, p0);
        if h != 0.0 {
            emit(h);
            emitted = true;
        }
        let (q_new, h) = fast_two_sum(p1, s);
        q = q_new;
        if h != 0.0 {
            emit(h);
            emitted = true;
        }
    }
    if q != 0.0 || !emitted {
        emit(q);
    }
}

/// The sign of a zero-eliminated expansion: the sign of its most significant component.
fn sign_of(components: &[f64]) -> Sign {
    components.last().map_or(Sign::Zero, |&x| Sign::of(x))
}

/// A stack expansion of at most `N` components. Constructors that combine expansions check at
/// compile time that `N` holds every component the operation can produce, so `push` never runs
/// out of room.
#[derive(Clone, Copy)]
pub(crate) struct Exp<const N: usize> {
    c: [f64; N],
    n: usize,
}

impl<const N: usize> Exp<N> {
    #[inline(always)]
    fn empty() -> Self {
        Self { c: [0.0; N], n: 0 }
    }

    #[inline(always)]
    fn push(&mut self, x: f64) {
        // In bounds by the compile-time capacity checks of the constructors.
        if let Some(slot) = self.c.get_mut(self.n) {
            *slot = x;
            self.n += 1;
        }
    }

    #[inline(always)]
    pub(crate) fn as_slice(&self) -> &[f64] {
        self.c.get(..self.n).unwrap_or(&[])
    }

    /// The expansion `hi + lo` produced by an error-free transformation (`lo` the roundoff).
    #[inline(always)]
    pub(crate) fn from_pair((hi, lo): (f64, f64)) -> Self {
        const { assert!(N >= 2) };
        let mut out = Self::empty();
        if lo != 0.0 {
            out.push(lo);
        }
        if hi != 0.0 || out.n == 0 {
            out.push(hi);
        }
        out
    }

    /// `e + f`.
    #[inline]
    pub(crate) fn sum<const A: usize, const B: usize>(e: &Exp<A>, f: &Exp<B>) -> Self {
        const { assert!(N >= A + B) };
        let mut out = Self::empty();
        sum_into(e.as_slice(), f.as_slice(), |x| out.push(x));
        out
    }

    /// `e * b`.
    #[inline]
    pub(crate) fn scale<const A: usize>(e: &Exp<A>, b: f64) -> Self {
        const { assert!(N >= 2 * A) };
        let mut out = Self::empty();
        scale_into(e.as_slice(), b, |x| out.push(x));
        out
    }

    /// `-e`, exactly.
    #[inline]
    pub(crate) fn negated(mut self) -> Self {
        if let Some(live) = self.c.get_mut(..self.n) {
            live.iter_mut().for_each(|x| *x = -*x);
        }
        self
    }

    /// An approximation of the value (Shewchuk's `estimate`): the sum of the components.
    #[inline]
    pub(crate) fn estimate(&self) -> f64 {
        self.as_slice().iter().sum()
    }

    pub(crate) fn sign(&self) -> Sign {
        sign_of(self.as_slice())
    }
}

/// The exact two-by-two product difference `a * b - c * d` as a four-component expansion
/// (Shewchuk's Two-Product followed by Two-Two-Diff).
#[inline]
pub(crate) fn cross_diff(a: f64, b: f64, c: f64, d: f64) -> Exp<4> {
    let ab = Exp::<2>::from_pair(two_product(a, b));
    let cd = Exp::<2>::from_pair(two_product(c, d)).negated();
    Exp::sum(&ab, &cd)
}

/// A heap expansion with exact ring operations, for the final exact stage of a predicate.
#[derive(Clone, Debug)]
pub(crate) struct Expansion(Vec<f64>);

impl Expansion {
    /// The exact difference `a - b`.
    pub(crate) fn diff(a: f64, b: f64) -> Self {
        Self(Exp::<2>::from_pair(two_diff(a, b)).as_slice().to_vec())
    }

    fn scaled(&self, b: f64) -> Self {
        let mut v = Vec::with_capacity(2 * self.0.len());
        scale_into(&self.0, b, |x| v.push(x));
        Self(v)
    }

    pub(crate) fn sign(&self) -> Sign {
        sign_of(&self.0)
    }
}

impl Add for Expansion {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let mut v = Vec::with_capacity(self.0.len() + rhs.0.len());
        sum_into(&self.0, &rhs.0, |x| v.push(x));
        Self(v)
    }
}

impl Neg for Expansion {
    type Output = Self;

    fn neg(mut self) -> Self {
        for x in self.0.iter_mut() {
            *x = -*x;
        }
        self
    }
}

impl Sub for Expansion {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self + (-rhs)
    }
}

impl Mul for Expansion {
    type Output = Self;

    /// The sum of `self` scaled by each component of `rhs` (distributivity; every partial
    /// product is exact).
    #[allow(clippy::suspicious_arithmetic_impl)] // the product is a sum of scaled expansions
    fn mul(self, rhs: Self) -> Self {
        let (long, short) = if self.0.len() >= rhs.0.len() { (self, rhs) } else { (rhs, self) };
        let mut terms = short.0.iter().map(|&b| long.scaled(b));
        let first = terms.next().unwrap_or_else(|| Self(vec![0.0]));
        terms.fold(first, |acc, term| acc + term)
    }
}

impl Ring for Expansion {
    fn from_f64(x: f64) -> Self {
        Self(vec![x])
    }
}

impl Signed for Expansion {
    fn sign(&self) -> Option<Sign> {
        Some(sign_of(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_and_products_are_exact() {
        let big = Expansion::from_f64(1e300);
        let tiny = Expansion::from_f64(1e-300);
        let s = big.clone() + tiny.clone();
        assert_eq!(s.0, vec![1e-300, 1e300]);
        assert_eq!((s - big).0, vec![1e-300]);
        let a = Expansion::diff(1.0, 1e-20); // 1 - 1e-20, two components
        let sq = a.clone() * a;
        // (1 - x)^2 = 1 - 2x + x^2 with x = 1e-20 (as doubles: the x^2 term is exact too).
        let expect = Expansion::from_f64(1.0) - Expansion::from_f64(2e-20) + Expansion::from_f64(1e-20) * Expansion::from_f64(1e-20);
        assert_eq!((sq - expect).sign(), Sign::Zero);
    }

    #[test]
    fn stack_and_heap_agree() {
        let e = cross_diff(3.0, 1e17, 1.0, 3e17 - 64.0);
        assert_eq!(e.estimate(), 64.0);
        let s = Exp::<8>::scale(&e, 1.0 + f64::EPSILON);
        assert_eq!(s.sign(), Sign::Positive);
        assert_eq!(Exp::<4>::from_pair((0.0, 0.0)).as_slice(), &[0.0]);
    }
}
