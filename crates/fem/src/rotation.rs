//! Incrementally objective stress updates for large deformation: the rotation that carries a
//! stress (and every other tensor of the material state) from one configuration to the next.
//!
//! Increments are given as the gradient of the displacement increment with respect to the
//! **midpoint configuration**, ΔG = ∂Δu/∂x_{n+½} (= Δt L at the midpoint). Its symmetric part is
//! the strain increment ΔD and its skew part the spin increment ΔW.
//!
//! **Jaumann rate, Hughes–Winget integration** (T. J. R. Hughes, J. Winget, *Finite rotation
//! effects in numerical integration of rate constitutive equations arising in large-deformation
//! analysis*, IJNME 15(12):1862–1867, 1980): the stress is rotated with
//! Q = (I − ½ΔW)⁻¹(I + ½ΔW) and the constitutive increment added: σ_{n+1} = Q σ_n Qᵀ + Δσ(ΔD).
//! Because ΔG is taken on the midpoint configuration, a rigid rotation x_{n+1} = R x_n gives
//! ΔG = 2(R − I)(R + I)⁻¹, which is skew (ΔD = 0), and its Cayley transform Q is R itself: a
//! rigid rotation of any size rotates the stress exactly and strains nothing.
//!
//! **Green–Naghdi rate, Flanagan–Taylor integration** (D. P. Flanagan, L. M. Taylor, *An
//! accurate numerical algorithm for stress integration with finite rotations*, CMAME
//! 62(3):305–320, 1987): the rotation R of the polar decomposition F = V R is integrated
//! incrementally instead of decomposing F. From V̇ = L V − V Ω with Ω = Ṙ Rᵀ and V symmetric,
//! Ω = W + X where the skew X solves V X + X V = D V − V D, that is, with axial vectors,
//! [(tr V) I − V] x = axial(D V − V D). R is advanced with the Cayley transform of Ω Δt, V with
//! V̇, and the material is updated in the unrotated frame: σ̂ = Rᵀ σ R, d̂ = Rᵀ D R. A material
//! whose axes are those of the undeformed body (orthotropic elasticity) is updated correctly in
//! that frame.
//!
//! Tensors are symmetric 6-vectors in the order xx, yy, zz, xy, xz, yz; strain-like vectors carry
//! engineering shears (γ = 2ε), stress-like ones tensor components.

use crate::dense::inverse3;

/// A 3 × 3 matrix, row-major.
pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut c = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            c[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    c
}

pub fn transpose(a: &Mat3) -> Mat3 {
    [[a[0][0], a[1][0], a[2][0]], [a[0][1], a[1][1], a[2][1]], [a[0][2], a[1][2], a[2][2]]]
}

/// A symmetric tensor from its 6-vector of tensor components.
pub fn tensor(s: &[f64; 6]) -> Mat3 {
    [[s[0], s[3], s[4]], [s[3], s[1], s[5]], [s[4], s[5], s[2]]]
}

/// The 6-vector of tensor components of (the symmetric part of) a matrix.
pub fn voigt(a: &Mat3) -> [f64; 6] {
    [a[0][0], a[1][1], a[2][2], 0.5 * (a[0][1] + a[1][0]), 0.5 * (a[0][2] + a[2][0]), 0.5 * (a[1][2] + a[2][1])]
}

/// Q s Qᵀ for a stress-like 6-vector.
pub fn rotate(q: &Mat3, s: &[f64; 6]) -> [f64; 6] {
    voigt(&mul(&mul(q, &tensor(s)), &transpose(q)))
}

/// Q e Qᵀ for a strain-like 6-vector (engineering shears).
pub fn rotate_strain(q: &Mat3, e: &[f64; 6]) -> [f64; 6] {
    let half = [e[0], e[1], e[2], 0.5 * e[3], 0.5 * e[4], 0.5 * e[5]];
    let r = rotate(q, &half);
    [r[0], r[1], r[2], 2.0 * r[3], 2.0 * r[4], 2.0 * r[5]]
}

/// Q v for a vector.
pub fn apply(q: &Mat3, v: &[f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| q[i][0] * v[0] + q[i][1] * v[1] + q[i][2] * v[2])
}

/// The strain increment ΔD (engineering shears) and the spin increment ΔW of a displacement
/// increment gradient.
pub fn split(g: &Mat3) -> ([f64; 6], Mat3) {
    let d = [g[0][0], g[1][1], g[2][2], g[0][1] + g[1][0], g[0][2] + g[2][0], g[1][2] + g[2][1]];
    let mut w = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            w[i][j] = 0.5 * (g[i][j] - g[j][i]);
        }
    }
    (d, w)
}

/// The Cayley transform (I − ½A)⁻¹(I + ½A) of a skew matrix: an exact rotation. `None` only
/// for non-finite input (I − ½A is invertible for every real skew A).
pub fn cayley(a: &Mat3) -> Option<Mat3> {
    let mut minus = IDENTITY;
    let mut plus = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            minus[i][j] -= 0.5 * a[i][j];
            plus[i][j] += 0.5 * a[i][j];
        }
    }
    let (inv, _) = inverse3(&minus)?;
    let q = mul(&inv, &plus);
    q.iter().flatten().all(|v| v.is_finite()).then_some(q)
}

/// The axial vector a of a skew matrix A (A v = a × v).
pub fn axial(a: &Mat3) -> [f64; 3] {
    [0.5 * (a[2][1] - a[1][2]), 0.5 * (a[0][2] - a[2][0]), 0.5 * (a[1][0] - a[0][1])]
}

/// The skew matrix of an axial vector.
pub fn skew(a: &[f64; 3]) -> Mat3 {
    [[0.0, -a[2], a[1]], [a[2], 0.0, -a[0]], [-a[1], a[0], 0.0]]
}

/// The polar rotation R and left stretch V (a 6-vector of tensor components) of the
/// Flanagan–Taylor integration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Polar {
    pub rotation: Mat3,
    pub stretch: [f64; 6],
}

impl Default for Polar {
    fn default() -> Polar {
        Polar { rotation: IDENTITY, stretch: [1.0, 1.0, 1.0, 0.0, 0.0, 0.0] }
    }
}

impl Polar {
    /// Advance R and V over an increment with displacement increment gradient `g` (on the
    /// midpoint configuration). V̇ = L V − V Ω is integrated with the midpoint rule (a predictor
    /// for V at the midpoint, then Ω and the increment from it), so V and R are second-order
    /// accurate in the increment. Returns the increment's rotation (R_{n+1} = q R_n); `None`
    /// when the increment is not finite or V degenerates.
    pub fn advance(&mut self, g: &Mat3) -> Option<Mat3> {
        let v0 = tensor(&self.stretch);
        let (dv, omega) = Polar::rate(g, &v0)?;
        let mut mid = v0;
        for i in 0..3 {
            for j in 0..3 {
                mid[i][j] += 0.5 * dv[i][j];
            }
        }
        let (dv, omega_mid) = Polar::rate(g, &mid).unwrap_or((dv, omega));
        let mut vn = v0;
        for i in 0..3 {
            for j in 0..3 {
                vn[i][j] += dv[i][j];
            }
        }
        let q = cayley(&omega_mid)?;
        self.rotation = mul(&q, &self.rotation);
        self.stretch = voigt(&vn);
        self.stretch.iter().all(|s| s.is_finite()).then_some(q)
    }

    /// The increments of V (L V − V Ω, times Δt) and the polar spin Ω Δt at stretch `v`.
    fn rate(g: &Mat3, v: &Mat3) -> Option<(Mat3, Mat3)> {
        let (d6, w) = split(g);
        let d = tensor(&[d6[0], d6[1], d6[2], 0.5 * d6[3], 0.5 * d6[4], 0.5 * d6[5]]);
        let (dv, vd) = (mul(&d, v), mul(v, &d));
        let mut rhs = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                rhs[i][j] = dv[i][j] - vd[i][j];
            }
        }
        let trace = v[0][0] + v[1][1] + v[2][2];
        let mut m = v.map(|r| r.map(|x| -x));
        for (i, row) in m.iter_mut().enumerate() {
            row[i] += trace;
        }
        let (inv, _) = inverse3(&m)?;
        let xs = skew(&apply(&inv, &axial(&rhs)));
        let mut omega = w;
        for i in 0..3 {
            for j in 0..3 {
                omega[i][j] += xs[i][j];
            }
        }
        let (lv, vo) = (mul(g, v), mul(v, &omega));
        let mut rate = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                rate[i][j] = lv[i][j] - vo[i][j];
            }
        }
        Some((rate, omega))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rotation_about(axis: [f64; 3], angle: f64) -> Mat3 {
        let n = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
        let k = skew(&axis.map(|c| c / n));
        let k2 = mul(&k, &k);
        let mut r = IDENTITY;
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] += angle.sin() * k[i][j] + (1.0 - angle.cos()) * k2[i][j];
            }
        }
        r
    }

    /// The midpoint gradient of the increment x → R x: 2 (R − I)(R + I)⁻¹.
    fn rigid_increment(r: &Mat3) -> Mat3 {
        let mut a = *r;
        let mut b = *r;
        for i in 0..3 {
            a[i][i] -= 1.0;
            b[i][i] += 1.0;
        }
        let (inv, _) = inverse3(&b).unwrap();
        mul(&a, &inv).map(|row| row.map(|v| 2.0 * v))
    }

    #[test]
    fn a_rigid_rotation_is_exact_with_both_rates() {
        let r = rotation_about([0.3, -0.5, 0.8], 1.1);
        let g = rigid_increment(&r);
        let (d, w) = split(&g);
        assert!(d.iter().all(|x| x.abs() < 1e-15), "{d:?}");
        let q = cayley(&w).unwrap();
        assert!(q.iter().flatten().zip(r.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-15), "{q:?} vs {r:?}");
        let mut p = Polar::default();
        p.advance(&g).unwrap();
        assert!(p.rotation.iter().flatten().zip(r.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-14));
        assert!(p.stretch.iter().zip([1.0, 1.0, 1.0, 0.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-14), "{:?}", p.stretch);
        let s = [1.0, 2.0, 3.0, 0.4, 0.5, 0.6];
        let rs = rotate(&q, &s);
        // Invariants: trace and the norm.
        let norm = |s: &[f64; 6]| s[..3].iter().map(|x| x * x).sum::<f64>() + 2.0 * s[3..].iter().map(|x| x * x).sum::<f64>();
        assert!((rs[0] + rs[1] + rs[2] - 6.0).abs() < 1e-14 && (norm(&rs) - norm(&s)).abs() < 1e-13);
    }

    #[test]
    fn polar_rotation_of_a_stretch_then_rotation() {
        // Stretch along x by 1.5 in many small steps, then rotate 0.7 rad about z in many small
        // steps: V = R diag(1.5, 1, 1) Rᵀ, R = rotation.
        let mut p = Polar::default();
        let n = 2000;
        let step = 1.5f64.powf(1.0 / n as f64);
        for _ in 0..n {
            // x_{n+1} = diag(step) x_n; midpoint gradient 2 (s − 1)/(s + 1).
            let mut g = [[0.0; 3]; 3];
            g[0][0] = 2.0 * (step - 1.0) / (step + 1.0);
            p.advance(&g).unwrap();
        }
        let r = rotation_about([0.0, 0.0, 1.0], 0.7 / n as f64);
        for _ in 0..n {
            p.advance(&rigid_increment(&r)).unwrap();
        }
        let want = rotation_about([0.0, 0.0, 1.0], 0.7);
        let v = rotate(&want, &[1.5, 1.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(p.rotation.iter().flatten().zip(want.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-9), "{:?}", p.rotation);
        assert!(p.stretch.iter().zip(v).all(|(a, b)| (a - b).abs() < 1e-6), "{:?} vs {v:?}", p.stretch);
    }
}
