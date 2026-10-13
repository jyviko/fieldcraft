//! Predicate formulas written once over exact or certified number types.
//!
//! A [`Ring`] is a number type with `+`, `-`, `*`; a [`Signed`] one can report a sign it is sure
//! of. The formulas below are instantiated with [`super::expansion::Expansion`] (the exact stage
//! of the explicit predicates), [`super::interval::Interval`] (the dynamic filter of the indirect
//! predicates) and [`super::dyadic::Dyadic`] (their exact stage). Writing each polynomial once
//! and letting the compiler monomorphize it plays the role of the code generators of Lévy's PCK
//! (2016) and Attene (2020, §6): one source formula, several arithmetic models.

use std::ops::{Add, Mul, Sub};

use super::Sign;

pub(crate) trait Ring: Clone + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> {
    /// The exact value of a double.
    fn from_f64(x: f64) -> Self;
}

pub(crate) trait Signed {
    /// The sign, when the representation is sure of it (always, for exact types).
    fn sign(&self) -> Option<Sign>;
}

/// `a * d - b * c`.
#[inline]
pub(crate) fn det2<T: Ring>(a: &T, b: &T, c: &T, d: &T) -> T {
    a.clone() * d.clone() - b.clone() * c.clone()
}

/// The determinant of the three rows, expanded along the last column.
#[inline]
pub(crate) fn det3<T: Ring>(rows: &[[T; 3]; 3]) -> T {
    let [[a0, a1, a2], [b0, b1, b2], [c0, c1, c2]] = rows;
    a2.clone() * det2(b0, b1, c0, c1) - b2.clone() * det2(a0, a1, c0, c1) + c2.clone() * det2(a0, a1, b0, b1)
}

/// `x² + y²` and `x² + y² + z²`.
#[inline]
pub(crate) fn sq2<T: Ring>(x: &T, y: &T) -> T {
    x.clone() * x.clone() + y.clone() * y.clone()
}

#[inline]
pub(crate) fn sq3<T: Ring>(x: &T, y: &T, z: &T) -> T {
    sq2(x, y) + z.clone() * z.clone()
}
