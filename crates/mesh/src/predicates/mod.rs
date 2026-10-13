//! Exact geometric predicates: the sign of `orient2d`, `orient3d`, `incircle` and `insphere`
//! for every admissible input, symbolic perturbation for the Delaunay tie-breaks, and indirect
//! predicates on points constructed as line–plane and three-plane intersections.
//!
//! # Sources
//! - J. R. Shewchuk, *Adaptive Precision Floating-Point Arithmetic and Fast Robust Geometric
//!   Predicates*, DCG 18:305–363, 1997: expansions (§2), adaptive evaluation (§3), the four
//!   predicates and their error bounds (§4). [`orient2d`], [`orient3d`], [`incircle`],
//!   [`insphere`] and their `*_filter` stage-A variants.
//! - O. Devillers, M. Teillaud, *Perturbations for Delaunay and weighted Delaunay 3D
//!   triangulations*, CGTA 44(3):160–168, 2011; H. Edelsbrunner, E. P. Mücke, *Simulation of
//!   Simplicity*, ACM TOG 9(1), 1990: [`insphere_sos`], [`incircle_sos`].
//! - M. Attene, *Indirect Predicates for Geometric Constructions*, CAD 126:102856, 2020 (paper
//!   only; his code is LGPL and is not read): [`Lpi`], [`Tpi`], [`GenericPoint`] and the
//!   `generic_*` predicates.
//!
//! # Arithmetic assumptions
//! IEEE 754 binary64, round-to-nearest-even, no extended intermediate precision, no FMA
//! contraction. Rust's `f64` gives exactly that on x86_64 (SSE2), aarch64 and
//! wasm32-unknown-unknown: rustc never fuses `a * b + c` and never uses x87 on these targets, and
//! the code uses no platform intrinsics, so every predicate returns the same sign on every
//! platform. The results are deterministic functions of the inputs.
//!
//! # Domain
//! Shewchuk's error bounds and the exactness of expansion arithmetic need no overflow and no
//! underflow (§5). Every coordinate of a [`Point2`] / [`Point3`] is therefore either zero or has
//! magnitude in [[`MIN_MAGNITUDE`], [`MAX_MAGNITUDE`]] = [2⁻¹²⁰, 2¹⁶⁰]: then every intermediate
//! value of the degree-5 `insphere` is either zero or above 2⁻⁸⁶⁰ and below 2⁸¹², all normal
//! doubles. The constructors reject anything else (NaN, infinities, out-of-range magnitudes) with
//! [`crate::Error::Invalid`]; the predicates themselves are total and never fail. Callers snap
//! tessellation noise (e.g. `1e-40`) to zero before building points.

mod adaptive;
mod dyadic;
mod eft;
mod expansion;
mod indirect;
mod interval;
mod ring;
mod sos;

pub use adaptive::{incircle, incircle_filter, insphere, insphere_filter, orient2d, orient2d_filter, orient3d, orient3d_filter};
pub use indirect::{Axis, GenericPoint, Lpi, Tpi, generic_compare, generic_orient2d, generic_orient3d};
pub use sos::{incircle_sos, insphere_sos};

use crate::{Error, Result};

/// The smallest nonzero coordinate magnitude a point may have: 2⁻¹²⁰ ≈ 7.5e-37.
pub const MIN_MAGNITUDE: f64 = f64::from_bits((1023 - 120) << 52);
/// The largest coordinate magnitude a point may have: 2¹⁶⁰ ≈ 1.5e48.
pub const MAX_MAGNITUDE: f64 = f64::from_bits((1023 + 160) << 52);

/// The sign of a predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sign {
    Negative = -1,
    Zero = 0,
    Positive = 1,
}

impl Sign {
    /// The sign of a double (`Zero` for both zeros; NaN, which the domain excludes, maps to
    /// `Zero`).
    #[inline(always)]
    pub fn of(x: f64) -> Sign {
        if x > 0.0 {
            Sign::Positive
        } else if x < 0.0 {
            Sign::Negative
        } else {
            Sign::Zero
        }
    }

    #[inline(always)]
    pub fn is_zero(self) -> bool {
        self == Sign::Zero
    }
}

impl std::ops::Neg for Sign {
    type Output = Sign;

    #[inline(always)]
    fn neg(self) -> Sign {
        match self {
            Sign::Negative => Sign::Positive,
            Sign::Zero => Sign::Zero,
            Sign::Positive => Sign::Negative,
        }
    }
}

impl std::ops::Mul for Sign {
    type Output = Sign;

    #[inline(always)]
    fn mul(self, rhs: Sign) -> Sign {
        match (self, rhs) {
            (Sign::Zero, _) | (_, Sign::Zero) => Sign::Zero,
            (a, b) if a == b => Sign::Positive,
            _ => Sign::Negative,
        }
    }
}

/// Whether `x` may be a point coordinate (see the module docs).
#[inline]
pub fn is_admissible(x: f64) -> bool {
    x == 0.0 || (x.abs() >= MIN_MAGNITUDE && x.abs() <= MAX_MAGNITUDE)
}

fn check<const N: usize>(c: [f64; N]) -> Result<[f64; N]> {
    match c.iter().find(|x| !is_admissible(**x)) {
        None => Ok(c),
        Some(x) => {
            Err(Error::Invalid(format!("point coordinate {x:e} is outside the exact-predicate domain (zero, or magnitude in [2^-120, 2^160])")))
        }
    }
}

/// A 2D point whose coordinates are admissible for the exact predicates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2([f64; 2]);

impl Point2 {
    pub fn new(x: f64, y: f64) -> Result<Self> {
        check([x, y]).map(Self)
    }

    #[inline(always)]
    pub fn coords(&self) -> [f64; 2] {
        self.0
    }
}

impl TryFrom<[f64; 2]> for Point2 {
    type Error = Error;

    fn try_from(c: [f64; 2]) -> Result<Self> {
        check(c).map(Self)
    }
}

/// A 3D point whose coordinates are admissible for the exact predicates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3([f64; 3]);

impl Point3 {
    pub fn new(x: f64, y: f64, z: f64) -> Result<Self> {
        check([x, y, z]).map(Self)
    }

    #[inline(always)]
    pub fn coords(&self) -> [f64; 3] {
        self.0
    }
}

impl TryFrom<[f64; 3]> for Point3 {
    type Error = Error;

    fn try_from(c: [f64; 3]) -> Result<Self> {
        check(c).map(Self)
    }
}

#[cfg(test)]
mod tests;
