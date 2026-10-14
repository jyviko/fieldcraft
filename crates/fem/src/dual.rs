//! Forward-mode automatic differentiation in [`VARS`] variables, for kernels whose exact first
//! and second derivatives are too long to write by hand (the mortar contact integrals of
//! [`crate::mortar`], whose segmentation moves with the nodes).
//!
//! A kernel is written once over a [`Real`] scalar and run on `f64` for its value, on [`Dual`]
//! for its gradient, or on [`Hyper`] for its gradient and Hessian. Arithmetic propagates the
//! derivatives by the chain rule (truncated Taylor arithmetic: A. Griewank, A. Walther,
//! *Evaluating Derivatives*, 2nd ed., SIAM 2008, ch. 3 and 13), so the derivatives are those of
//! the code as it runs, to round-off: the branches a kernel takes (which polygon edges clip,
//! which root of a quadratic) are decided on the values and differentiated where they are
//! taken, which is the derivative of the piecewise smooth function wherever it is smooth. The
//! automated linearisation of contact kernels this way is that of J. Lengiewicz, J. Korelc, S.
//! Stupkiewicz, *Automation of finite element formulations for large deformation contact
//! problems*, IJNME 85:1252–1279, 2011.
//!
//! [`jet::Jet`](crate::jet::Jet) is the three-variable sibling the shape-function library uses.

use std::ops::{Add, Div, Mul, Neg, Sub};

/// Independent variables of [`Dual`] and [`Hyper`]: the corners of two faces of up to four
/// nodes, three coordinates each.
pub const VARS: usize = 24;

/// Entries of a packed symmetric [`VARS`] × [`VARS`] Hessian (its upper triangle by rows).
pub const PACKED: usize = VARS * (VARS + 1) / 2;

/// The position of (i, j) in a packed upper triangle (either order).
pub fn packed(i: usize, j: usize) -> usize {
    let (a, b) = if i <= j { (i, j) } else { (j, i) };
    // Row a starts after rows 0..a, of VARS, VARS − 1, … entries.
    a * VARS - a * a.saturating_sub(1) / 2 + (b - a)
}

/// A scalar a kernel can be written over: `f64`, [`Dual`] or [`Hyper`].
pub trait Real:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + Add<f64, Output = Self>
    + Sub<f64, Output = Self>
    + Mul<f64, Output = Self>
{
    /// A constant.
    fn cst(v: f64) -> Self;
    /// The value.
    fn val(&self) -> f64;
    fn sqrt(self) -> Self;
}

impl Real for f64 {
    fn cst(v: f64) -> f64 {
        v
    }
    fn val(&self) -> f64 {
        *self
    }
    fn sqrt(self) -> f64 {
        f64::sqrt(self)
    }
}

/// A value and its gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dual {
    pub v: f64,
    pub g: [f64; VARS],
}

impl Dual {
    /// Variable `k` at value `v` (a constant when `k` is out of range).
    pub fn var(v: f64, k: usize) -> Dual {
        let mut d = Dual::cst(v);
        if let Some(g) = d.g.get_mut(k) {
            *g = 1.0;
        }
        d
    }

    fn chain(self, v: f64, d: f64) -> Dual {
        Dual { v, g: self.g.map(|x| d * x) }
    }
}

impl Real for Dual {
    fn cst(v: f64) -> Dual {
        Dual { v, g: [0.0; VARS] }
    }
    fn val(&self) -> f64 {
        self.v
    }
    fn sqrt(self) -> Dual {
        let s = self.v.sqrt();
        self.chain(s, 0.5 / s)
    }
}

impl Add for Dual {
    type Output = Dual;
    fn add(mut self, o: Dual) -> Dual {
        self.v += o.v;
        self.g.iter_mut().zip(o.g).for_each(|(a, b)| *a += b);
        self
    }
}

impl Sub for Dual {
    type Output = Dual;
    fn sub(mut self, o: Dual) -> Dual {
        self.v -= o.v;
        self.g.iter_mut().zip(o.g).for_each(|(a, b)| *a -= b);
        self
    }
}

impl Mul for Dual {
    type Output = Dual;
    fn mul(self, o: Dual) -> Dual {
        let mut g = [0.0; VARS];
        for k in 0..VARS {
            g[k] = self.v * o.g[k] + o.v * self.g[k];
        }
        Dual { v: self.v * o.v, g }
    }
}

impl Div for Dual {
    type Output = Dual;
    fn div(self, o: Dual) -> Dual {
        let r = 1.0 / o.v;
        let v = self.v * r;
        let mut g = [0.0; VARS];
        for k in 0..VARS {
            g[k] = (self.g[k] - v * o.g[k]) * r;
        }
        Dual { v, g }
    }
}

impl Neg for Dual {
    type Output = Dual;
    fn neg(self) -> Dual {
        Dual { v: -self.v, g: self.g.map(|x| -x) }
    }
}

impl Add<f64> for Dual {
    type Output = Dual;
    fn add(mut self, o: f64) -> Dual {
        self.v += o;
        self
    }
}

impl Sub<f64> for Dual {
    type Output = Dual;
    fn sub(mut self, o: f64) -> Dual {
        self.v -= o;
        self
    }
}

impl Mul<f64> for Dual {
    type Output = Dual;
    fn mul(self, o: f64) -> Dual {
        Dual { v: self.v * o, g: self.g.map(|x| x * o) }
    }
}

/// A value, its gradient and its Hessian (packed upper triangle, [`packed`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hyper {
    pub v: f64,
    pub g: [f64; VARS],
    pub h: [f64; PACKED],
}

impl Hyper {
    /// Variable `k` at value `v` (a constant when `k` is out of range).
    pub fn var(v: f64, k: usize) -> Hyper {
        let mut d = Hyper::cst(v);
        if let Some(g) = d.g.get_mut(k) {
            *g = 1.0;
        }
        d
    }

    /// ∂²/∂x_i∂x_j.
    pub fn hessian(&self, i: usize, j: usize) -> f64 {
        self.h.get(packed(i, j)).copied().unwrap_or(0.0)
    }

    /// f(self) from f's value `f0`, first derivative `f1` and second derivative `f2` at the
    /// value: g = f1 g, H = f1 H + f2 g gᵀ.
    fn chain(&self, f0: f64, f1: f64, f2: f64) -> Hyper {
        let mut out = Hyper { v: f0, g: self.g.map(|x| f1 * x), h: [0.0; PACKED] };
        let mut at = 0;
        for i in 0..VARS {
            let gi = f2 * self.g[i];
            for j in i..VARS {
                out.h[at] = f1 * self.h[at] + gi * self.g[j];
                at += 1;
            }
        }
        out
    }
}

impl Real for Hyper {
    fn cst(v: f64) -> Hyper {
        Hyper { v, g: [0.0; VARS], h: [0.0; PACKED] }
    }
    fn val(&self) -> f64 {
        self.v
    }
    fn sqrt(self) -> Hyper {
        let s = self.v.sqrt();
        self.chain(s, 0.5 / s, -0.25 / (s * s * s))
    }
}

impl Add for Hyper {
    type Output = Hyper;
    fn add(mut self, o: Hyper) -> Hyper {
        self.v += o.v;
        self.g.iter_mut().zip(&o.g).for_each(|(a, b)| *a += b);
        self.h.iter_mut().zip(&o.h).for_each(|(a, b)| *a += b);
        self
    }
}

impl Sub for Hyper {
    type Output = Hyper;
    fn sub(mut self, o: Hyper) -> Hyper {
        self.v -= o.v;
        self.g.iter_mut().zip(&o.g).for_each(|(a, b)| *a -= b);
        self.h.iter_mut().zip(&o.h).for_each(|(a, b)| *a -= b);
        self
    }
}

impl Mul for Hyper {
    type Output = Hyper;
    fn mul(self, o: Hyper) -> Hyper {
        let mut out = Hyper { v: self.v * o.v, g: [0.0; VARS], h: [0.0; PACKED] };
        for k in 0..VARS {
            out.g[k] = self.v * o.g[k] + o.v * self.g[k];
        }
        let mut at = 0;
        for i in 0..VARS {
            let (a, b) = (self.g[i], o.g[i]);
            for j in i..VARS {
                out.h[at] = self.v * o.h[at] + o.v * self.h[at] + a * o.g[j] + b * self.g[j];
                at += 1;
            }
        }
        out
    }
}

impl Div for Hyper {
    type Output = Hyper;
    fn div(self, o: Hyper) -> Hyper {
        let r = 1.0 / o.v;
        self * o.chain(r, -r * r, 2.0 * r * r * r)
    }
}

impl Neg for Hyper {
    type Output = Hyper;
    fn neg(self) -> Hyper {
        Hyper { v: -self.v, g: self.g.map(|x| -x), h: self.h.map(|x| -x) }
    }
}

impl Add<f64> for Hyper {
    type Output = Hyper;
    fn add(mut self, o: f64) -> Hyper {
        self.v += o;
        self
    }
}

impl Sub<f64> for Hyper {
    type Output = Hyper;
    fn sub(mut self, o: f64) -> Hyper {
        self.v -= o;
        self
    }
}

impl Mul<f64> for Hyper {
    type Output = Hyper;
    fn mul(self, o: f64) -> Hyper {
        Hyper { v: self.v * o, g: self.g.map(|x| x * o), h: self.h.map(|x| x * o) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f<T: Real>(x: T, y: T) -> T {
        (x * x * y + T::cst(1.0)).sqrt() / (y - x * 0.5) + x * 3.0
    }

    #[test]
    fn packed_indices_cover_the_upper_triangle_once() {
        let mut seen = vec![false; PACKED];
        let mut at = 0;
        for i in 0..VARS {
            for j in i..VARS {
                assert_eq!(packed(i, j), at);
                assert_eq!(packed(j, i), at);
                seen[at] = true;
                at += 1;
            }
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn derivatives_match_finite_differences() {
        let (x, y) = (0.7, 2.3);
        let h = 1e-5;
        let fx = |x: f64, y: f64| f(x, y);
        let gx = (fx(x + h, y) - fx(x - h, y)) / (2.0 * h);
        let gy = (fx(x, y + h) - fx(x, y - h)) / (2.0 * h);
        let hxy = (fx(x + h, y + h) - fx(x + h, y - h) - fx(x - h, y + h) + fx(x - h, y - h)) / (4.0 * h * h);
        let hxx = (fx(x + h, y) - 2.0 * fx(x, y) + fx(x - h, y)) / (h * h);
        let d = f(Dual::var(x, 3), Dual::var(y, 17));
        assert!((d.v - fx(x, y)).abs() < 1e-14 && (d.g[3] - gx).abs() < 1e-8 && (d.g[17] - gy).abs() < 1e-8);
        let e = f(Hyper::var(x, 3), Hyper::var(y, 17));
        assert!((e.g[3] - gx).abs() < 1e-8 && (e.g[17] - gy).abs() < 1e-8);
        assert!((e.hessian(3, 17) - hxy).abs() < 1e-4 && (e.hessian(17, 3) - hxy).abs() < 1e-4, "{} {hxy}", e.hessian(3, 17));
        assert!((e.hessian(3, 3) - hxx).abs() < 1e-3);
        assert_eq!(e.hessian(0, 5), 0.0);
    }
}
