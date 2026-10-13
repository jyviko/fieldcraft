//! Second-order forward-mode automatic differentiation in three variables.
//!
//! A [`Jet`] carries a value with its gradient and Hessian with respect to the natural
//! coordinates (ξ, η, ζ). Arithmetic propagates them by the product and quotient rules, so a
//! shape function written once as a formula yields its exact first and second derivatives (to
//! round-off, no finite differences): the hyper-dual / truncated Taylor arithmetic of A. Griewank,
//! A. Walther, *Evaluating Derivatives*, 2nd ed., SIAM 2008, ch. 13.

use std::ops::{Add, Div, Mul, Neg, Sub};

/// A value, its gradient and its (symmetric) Hessian.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jet {
    pub v: f64,
    pub g: [f64; 3],
    pub h: [[f64; 3]; 3],
}

impl Jet {
    pub const fn constant(v: f64) -> Jet {
        Jet { v, g: [0.0; 3], h: [[0.0; 3]; 3] }
    }

    /// The independent variable `k` (0–2) at value `v`.
    pub fn var(v: f64, k: usize) -> Jet {
        let mut j = Jet::constant(v);
        if let Some(g) = j.g.get_mut(k) {
            *g = 1.0;
        }
        j
    }

    /// The three natural coordinates as variables.
    pub fn point(xi: [f64; 3]) -> [Jet; 3] {
        [Jet::var(xi[0], 0), Jet::var(xi[1], 1), Jet::var(xi[2], 2)]
    }

    /// 1 / self; the Hessian term follows from (1/b)'' = 2 b'b'ᵀ / b³ − b'' / b².
    pub fn recip(self) -> Jet {
        let r = 1.0 / self.v;
        let r2 = r * r;
        let mut out = Jet::constant(r);
        for a in 0..3 {
            out.g[a] = -self.g[a] * r2;
            for b in 0..3 {
                out.h[a][b] = 2.0 * self.g[a] * self.g[b] * r2 * r - self.h[a][b] * r2;
            }
        }
        out
    }

    fn map(self, other: Jet, f: impl Fn(f64, f64) -> f64) -> Jet {
        let mut out = Jet::constant(f(self.v, other.v));
        for a in 0..3 {
            out.g[a] = f(self.g[a], other.g[a]);
            for b in 0..3 {
                out.h[a][b] = f(self.h[a][b], other.h[a][b]);
            }
        }
        out
    }

    fn scale(self, s: f64) -> Jet {
        Jet { v: self.v * s, g: self.g.map(|x| x * s), h: self.h.map(|r| r.map(|x| x * s)) }
    }
}

impl Add for Jet {
    type Output = Jet;
    fn add(self, o: Jet) -> Jet {
        self.map(o, |a, b| a + b)
    }
}

impl Sub for Jet {
    type Output = Jet;
    fn sub(self, o: Jet) -> Jet {
        self.map(o, |a, b| a - b)
    }
}

impl Neg for Jet {
    type Output = Jet;
    fn neg(self) -> Jet {
        self.scale(-1.0)
    }
}

impl Mul for Jet {
    type Output = Jet;
    fn mul(self, o: Jet) -> Jet {
        let mut out = Jet::constant(self.v * o.v);
        for a in 0..3 {
            out.g[a] = self.v * o.g[a] + o.v * self.g[a];
            for b in 0..3 {
                out.h[a][b] = self.v * o.h[a][b] + o.v * self.h[a][b] + self.g[a] * o.g[b] + o.g[a] * self.g[b];
            }
        }
        out
    }
}

impl Div for Jet {
    type Output = Jet;
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, o: Jet) -> Jet {
        self * o.recip()
    }
}

impl Add<f64> for Jet {
    type Output = Jet;
    fn add(mut self, c: f64) -> Jet {
        self.v += c;
        self
    }
}

impl Sub<f64> for Jet {
    type Output = Jet;
    fn sub(mut self, c: f64) -> Jet {
        self.v -= c;
        self
    }
}

impl Mul<f64> for Jet {
    type Output = Jet;
    fn mul(self, c: f64) -> Jet {
        self.scale(c)
    }
}

impl Add<Jet> for f64 {
    type Output = Jet;
    fn add(self, j: Jet) -> Jet {
        j + self
    }
}

impl Sub<Jet> for f64 {
    type Output = Jet;
    fn sub(self, j: Jet) -> Jet {
        -j + self
    }
}

impl Mul<Jet> for f64 {
    type Output = Jet;
    fn mul(self, j: Jet) -> Jet {
        j.scale(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn products_and_quotients_differentiate_exactly() {
        let [x, y, z] = Jet::point([0.3, -0.7, 1.9]);
        // f = x² y / (1 + z) + 3 x z.
        let f = x * x * y / (1.0 + z) + 3.0 * x * z;
        let (xv, yv, zv) = (0.3, -0.7, 1.9);
        let d = 1.0 + zv;
        assert!((f.v - (xv * xv * yv / d + 3.0 * xv * zv)).abs() < 1e-15);
        let g = [2.0 * xv * yv / d + 3.0 * zv, xv * xv / d, -xv * xv * yv / (d * d) + 3.0 * xv];
        let h = [
            [2.0 * yv / d, 2.0 * xv / d, -2.0 * xv * yv / (d * d) + 3.0],
            [2.0 * xv / d, 0.0, -xv * xv / (d * d)],
            [-2.0 * xv * yv / (d * d) + 3.0, -xv * xv / (d * d), 2.0 * xv * xv * yv / (d * d * d)],
        ];
        for a in 0..3 {
            assert!((f.g[a] - g[a]).abs() < 1e-14, "g{a}");
            for b in 0..3 {
                assert!((f.h[a][b] - h[a][b]).abs() < 1e-14, "h{a}{b}: {} vs {}", f.h[a][b], h[a][b]);
            }
        }
        let s = 2.0 - x * 0.5 - (y - 1.0);
        assert_eq!(s.g, [-0.5, -1.0, 0.0]);
        assert_eq!(s.v, 2.0 - 0.15 + 1.7);
    }
}
