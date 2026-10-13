//! Linear elastic constitutive matrices.
//!
//! Stress and strain are 6-vectors in the order xx, yy, zz, xy, xz, yz, with engineering shear
//! strains (γ = 2ε), so σ = D ε. The order puts the orthotropic shear moduli G₁₂, G₁₃, G₂₃ on
//! the diagonal in the order the model lists them. Material axes are the global axes until the
//! model has material orientations.
//!
//! Orthotropic stiffness is the inverse of the compliance written with engineering constants
//! (R. M. Jones, *Mechanics of Composite Materials*, 2nd ed., Taylor & Francis 1999, §2.4):
//! S₁₁ = 1/E₁, S₁₂ = −ν₁₂/E₁, S₁₃ = −ν₁₃/E₁, S₂₃ = −ν₂₃/E₂ (ν_ji/E_j = ν_ij/E_i), and
//! 1/G on the shear diagonal.

use modelcraft_model::Elastic;

use crate::dense::inverse3;
use crate::{Error, Result};

/// A 6 × 6 elasticity matrix, row-major.
pub type Elasticity = [[f64; 6]; 6];

/// The isotropic elasticity matrix.
pub fn isotropic(young: f64, poisson: f64) -> Result<Elasticity> {
    if !(young > 0.0 && young.is_finite() && poisson > -1.0 && poisson < 0.5) {
        return Err(Error::Invalid(format!("isotropic constants E = {young}, ν = {poisson} are not a stable material")));
    }
    let lambda = young * poisson / ((1.0 + poisson) * (1.0 - 2.0 * poisson));
    let mu = young / (2.0 * (1.0 + poisson));
    let mut d = [[0.0; 6]; 6];
    for i in 0..3 {
        for j in 0..3 {
            d[i][j] = lambda;
        }
        d[i][i] = lambda + 2.0 * mu;
        d[i + 3][i + 3] = mu;
    }
    Ok(d)
}

/// The orthotropic elasticity matrix from engineering constants.
pub fn orthotropic(young: [f64; 3], poisson: [f64; 3], shear: [f64; 3]) -> Result<Elasticity> {
    let [e1, e2, e3] = young;
    let [n12, n13, n23] = poisson;
    let s = [[1.0 / e1, -n12 / e1, -n13 / e1], [-n12 / e1, 1.0 / e2, -n23 / e2], [-n13 / e1, -n23 / e2, 1.0 / e3]];
    let scale = (1.0 / e1) * (1.0 / e2) * (1.0 / e3);
    let inverse = inverse3(&s).filter(|(_, det)| *det > 1e-12 * scale);
    let Some((normal, _)) = inverse.filter(|_| shear.iter().all(|g| *g > 0.0 && g.is_finite())) else {
        return Err(Error::Invalid("the orthotropic constants do not describe a stable material".into()));
    };
    let mut d = [[0.0; 6]; 6];
    for i in 0..3 {
        d[i][..3].copy_from_slice(&normal[i]);
    }
    for (k, g) in shear.iter().enumerate() {
        d[3 + k][3 + k] = *g;
    }
    Ok(d)
}

/// The elasticity matrix of a model material's elastic behaviour.
pub fn elasticity(e: &Elastic) -> Result<Elasticity> {
    match *e {
        Elastic::Isotropic { young, poisson } => isotropic(young, poisson),
        Elastic::EngineeringConstants { young, poisson, shear } => orthotropic(young, poisson, shear),
    }
}

/// The von Mises equivalent stress of a stress 6-vector.
pub fn von_mises(s: &[f64; 6]) -> f64 {
    let [xx, yy, zz, xy, xz, yz] = *s;
    let normal = (xx - yy) * (xx - yy) + (yy - zz) * (yy - zz) + (zz - xx) * (zz - xx);
    (0.5 * normal + 3.0 * (xy * xy + xz * xz + yz * yz)).sqrt()
}

/// σ = D ε.
pub fn stress(d: &Elasticity, strain: &[f64; 6]) -> [f64; 6] {
    let mut s = [0.0; 6];
    for (si, row) in s.iter_mut().zip(d) {
        *si = row.iter().zip(strain).map(|(a, b)| a * b).sum();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthotropic_with_isotropic_constants_is_isotropic() {
        let (e, nu) = (210e3, 0.3);
        let g = e / (2.0 * (1.0 + nu));
        let a = isotropic(e, nu).unwrap();
        let b = orthotropic([e; 3], [nu; 3], [g; 3]).unwrap();
        for i in 0..6 {
            for j in 0..6 {
                assert!((a[i][j] - b[i][j]).abs() < 1e-9 * e, "{i} {j}: {} vs {}", a[i][j], b[i][j]);
            }
        }
        assert!(isotropic(1.0, 0.5).is_err());
        assert!(orthotropic([1.0; 3], [0.9; 3], [1.0; 3]).is_err());
    }

    #[test]
    fn orthotropic_uniaxial_response() {
        // Stress along 1 alone gives strains σ/E1, −ν12 σ/E1, −ν13 σ/E1.
        let (young, poisson) = ([140e3, 10e3, 12e3], [0.3, 0.25, 0.4]);
        let d = orthotropic(young, poisson, [5e3, 4e3, 3.8e3]).unwrap();
        let strain = [1.0 / young[0], -poisson[0] / young[0], -poisson[1] / young[0], 0.0, 0.0, 0.0];
        let s = stress(&d, &strain);
        assert!((s[0] - 1.0).abs() < 1e-12 && s[1].abs() < 1e-12 && s[2].abs() < 1e-12, "{s:?}");
        assert!((von_mises(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]) - 1.0).abs() < 1e-15);
        assert!((von_mises(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]) - 3f64.sqrt()).abs() < 1e-15);
    }
}
