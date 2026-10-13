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

/// The determinant of four rows `[x, y, z, w]`, expanded along the last column.
pub(crate) fn det4<T: Ring>(rows: &[[T; 4]; 4]) -> T {
    let minor = |skip: usize| -> [[T; 3]; 3] {
        let mut kept = rows.iter().enumerate().filter(|(i, _)| *i != skip).map(|(_, [x, y, z, _])| [x.clone(), y.clone(), z.clone()]);
        let zero = || [T::from_f64(0.0), T::from_f64(0.0), T::from_f64(0.0)];
        [kept.next().unwrap_or_else(zero), kept.next().unwrap_or_else(zero), kept.next().unwrap_or_else(zero)]
    };
    let [[.., w0], [.., w1], [.., w2], [.., w3]] = rows;
    // Cofactor signs (-1)^(i + 4) for rows i = 1..4.
    w1.clone() * det3(&minor(1)) - w0.clone() * det3(&minor(0)) + w3.clone() * det3(&minor(3)) - w2.clone() * det3(&minor(2))
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
