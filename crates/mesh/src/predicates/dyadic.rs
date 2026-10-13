//! Exact dyadic rationals `±m · 2^e` (`m` an arbitrary-precision natural number), the exact
//! stage of the indirect predicates.
//!
//! Attene evaluates this stage with floating-point expansions (2020, §5.3). Expansions are exact
//! only while no partial product underflows, and the polynomials of indirect predicates reach
//! degree 27 (`orient3d` of four three-plane intersections), far beyond what the coordinate
//! domain keeps out of underflow. Integer arithmetic has no such limit: every double is a dyadic
//! rational, and sums and products of dyadic rationals are computed exactly. The stage only runs
//! when the interval filter cannot decide, so its speed matters little.

use std::cmp::Ordering;
use std::ops::{Add, Mul, Neg, Sub};

use super::Sign;
use super::ring::{Ring, Signed};

/// `(-1)^neg · mag · 2^exp`; `mag` holds little-endian base-2³² limbs with neither leading nor
/// trailing zero limbs, and zero is the empty magnitude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dyadic {
    neg: bool,
    mag: Vec<u32>,
    exp: i64,
}

impl Dyadic {
    fn zero() -> Self {
        Self { neg: false, mag: Vec::new(), exp: 0 }
    }

    fn normalized(neg: bool, mut mag: Vec<u32>, mut exp: i64) -> Self {
        while mag.last() == Some(&0) {
            mag.pop();
        }
        let low_zeros = mag.iter().take_while(|&&l| l == 0).count();
        if low_zeros > 0 {
            mag.drain(..low_zeros);
            exp = exp.saturating_add(32 * low_zeros as i64);
        }
        if mag.is_empty() { Self::zero() } else { Self { neg, mag, exp } }
    }

    fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    /// The magnitude shifted left by `bits`.
    fn shifted(&self, bits: u64) -> Vec<u32> {
        let limbs = (bits / 32) as usize;
        let bits = (bits % 32) as u32;
        let mut out = vec![0u32; limbs];
        out.reserve(self.mag.len() + 1);
        if bits == 0 {
            out.extend_from_slice(&self.mag);
        } else {
            let mut carry = 0u32;
            for &l in &self.mag {
                out.push((l << bits) | carry);
                carry = l >> (32 - bits);
            }
            out.push(carry);
        }
        out
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    let (a, b) = (trimmed(a), trimmed(b));
    a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn trimmed(a: &[u32]) -> &[u32] {
    let len = a.iter().rposition(|&l| l != 0).map_or(0, |i| i + 1);
    a.get(..len).unwrap_or(a)
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for (i, &l) in long.iter().enumerate() {
        let t = l as u64 + short.get(i).copied().unwrap_or(0) as u64 + carry;
        out.push(t as u32);
        carry = t >> 32;
    }
    out.push(carry as u32);
    out
}

/// `a - b` for `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &l) in a.iter().enumerate() {
        let mut t = l as i64 - b.get(i).copied().unwrap_or(0) as i64 - borrow;
        borrow = if t < 0 {
            t += 1 << 32;
            1
        } else {
            0
        };
        out.push(t as u32);
    }
    out
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (slot, &y) in out.iter_mut().skip(i).zip(b) {
            // x·y + slot + carry ≤ (2³² − 1)² + 2(2³² − 1) = 2⁶⁴ − 1: no overflow.
            let t = x as u64 * y as u64 + *slot as u64 + carry;
            *slot = t as u32;
            carry = t >> 32;
        }
        if let Some(slot) = out.get_mut(i + b.len()) {
            *slot = carry as u32;
        }
    }
    out
}

impl Ring for Dyadic {
    fn from_f64(x: f64) -> Self {
        let bits = x.to_bits();
        let neg = bits >> 63 == 1;
        let biased = ((bits >> 52) & 0x7ff) as i64;
        let fraction = bits & ((1u64 << 52) - 1);
        let (mant, exp) = match biased {
            0 => (fraction, -1074),
            // Not finite: unreachable for admissible coordinates; map to zero to stay total.
            0x7ff => return Self::zero(),
            _ => (fraction | (1u64 << 52), biased - 1075),
        };
        Self::normalized(neg, vec![mant as u32, (mant >> 32) as u32], exp)
    }
}

impl Neg for Dyadic {
    type Output = Self;

    fn neg(mut self) -> Self {
        if !self.is_zero() {
            self.neg = !self.neg;
        }
        self
    }
}

impl Add for Dyadic {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        if self.is_zero() {
            return rhs;
        }
        if rhs.is_zero() {
            return self;
        }
        let exp = self.exp.min(rhs.exp);
        // Exponents stay far from i64 limits (bounded by polynomial degree × 1100), so these
        // differences are small and non-negative.
        let a = self.shifted(self.exp.saturating_sub(exp).unsigned_abs());
        let b = rhs.shifted(rhs.exp.saturating_sub(exp).unsigned_abs());
        if self.neg == rhs.neg {
            return Self::normalized(self.neg, add_mag(&a, &b), exp);
        }
        match cmp_mag(&a, &b) {
            Ordering::Equal => Self::zero(),
            Ordering::Greater => Self::normalized(self.neg, sub_mag(&a, &b), exp),
            Ordering::Less => Self::normalized(rhs.neg, sub_mag(&b, &a), exp),
        }
    }
}

impl Sub for Dyadic {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self + (-rhs)
    }
}

impl Mul for Dyadic {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        if self.is_zero() || rhs.is_zero() {
            return Self::zero();
        }
        Self::normalized(self.neg != rhs.neg, mul_mag(&self.mag, &rhs.mag), self.exp.saturating_add(rhs.exp))
    }
}

impl Signed for Dyadic {
    fn sign(&self) -> Option<Sign> {
        Some(if self.is_zero() {
            Sign::Zero
        } else if self.neg {
            Sign::Negative
        } else {
            Sign::Positive
        })
    }
}

#[cfg(test)]
mod tests {
    use num_bigint::BigInt;
    use num_rational::BigRational;
    use num_traits::{FromPrimitive, One};

    use super::*;

    fn to_rational(d: &Dyadic) -> BigRational {
        let mut m = BigInt::from(0);
        for &l in d.mag.iter().rev() {
            m = (m << 32) + BigInt::from(l);
        }
        if d.neg {
            m = -m;
        }
        let two = BigRational::from_integer(BigInt::from(2));
        let r = BigRational::from_integer(m);
        let mut scale = BigRational::one();
        for _ in 0..d.exp.unsigned_abs() {
            scale *= two.clone();
        }
        if d.exp >= 0 { r * scale } else { r / scale }
    }

    fn q(x: f64) -> BigRational {
        BigRational::from_f64(x).unwrap()
    }

    #[test]
    fn matches_big_rationals() {
        let xs = [0.0, -0.0, 1.0, -3.5, 1e300, -1e-300, 5e-324, 0.1, f64::MAX, 7.0e15 + 1.0, -2.0f64.powi(-1000)];
        for &a in &xs {
            assert_eq!(to_rational(&Dyadic::from_f64(a)), q(a), "{a}");
            for &b in &xs {
                let (da, db) = (Dyadic::from_f64(a), Dyadic::from_f64(b));
                assert_eq!(to_rational(&(da.clone() + db.clone())), q(a) + q(b), "{a} + {b}");
                assert_eq!(to_rational(&(da.clone() - db.clone())), q(a) - q(b), "{a} - {b}");
                assert_eq!(to_rational(&(da * db)), q(a) * q(b), "{a} * {b}");
            }
        }
        let third = Dyadic::from_f64(1.0 / 3.0);
        let s = third.clone() + third.clone() + third - Dyadic::from_f64(1.0);
        assert_eq!(s.sign(), Some(Sign::Negative)); // fl(1/3) < 1/3
    }
}
