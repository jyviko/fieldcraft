//! Small dense matrices: element matrices and the little systems inside an element (static
//! condensation, extrapolation fits).

use crate::{Error, Result};

/// A square matrix, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub n: usize,
    pub data: Vec<f64>,
}

impl Matrix {
    pub fn zeros(n: usize) -> Matrix {
        Matrix { n, data: vec![0.0; n * n] }
    }

    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.data.get(i * self.n + j).copied().unwrap_or(0.0)
    }

    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        if let Some(x) = self.data.get_mut(i * self.n + j) {
            *x += v;
        }
    }

    /// y = A x (x shorter than n is padded with zeros).
    pub fn mul_vec(&self, x: &[f64]) -> Vec<f64> {
        self.data.chunks_exact(self.n.max(1)).map(|row| row.iter().zip(x).map(|(a, b)| a * b).sum()).collect()
    }
}

/// The Cholesky factor of a small symmetric positive definite matrix (lower triangle, row-major).
#[derive(Debug, Clone)]
pub struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    /// Factor `a` (only its lower triangle is read). Fails when a pivot is not positive relative
    /// to the matrix's diagonal.
    pub fn new(a: &Matrix) -> Result<Cholesky> {
        let n = a.n;
        let mut l = vec![0.0; n * n];
        let scale = (0..n).map(|i| a.get(i, i).abs()).fold(0.0, f64::max);
        for j in 0..n {
            let mut d = a.get(j, j);
            for k in 0..j {
                let v = l[j * n + k];
                d -= v * v;
            }
            if !(d > 1e-13 * scale) {
                return Err(Error::Singular(format!("a {n} × {n} element matrix is not positive definite (pivot {j})")));
            }
            let djj = d.sqrt();
            l[j * n + j] = djj;
            for i in j + 1..n {
                let mut s = a.get(i, j);
                for k in 0..j {
                    s -= l[i * n + k] * l[j * n + k];
                }
                l[i * n + j] = s / djj;
            }
        }
        Ok(Cholesky { n, l })
    }

    /// Solve A x = b in place.
    pub fn solve(&self, b: &mut [f64]) {
        let n = self.n.min(b.len());
        for i in 0..n {
            let mut s = b[i];
            for k in 0..i {
                s -= self.l[i * self.n + k] * b[k];
            }
            b[i] = s / self.l[i * self.n + i];
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in i + 1..n {
                s -= self.l[k * self.n + i] * b[k];
            }
            b[i] = s / self.l[i * self.n + i];
        }
    }
}

/// The inverse and determinant of a 3 × 3 matrix; `None` when it is singular or not finite.
pub fn inverse3(m: &[[f64; 3]; 3]) -> Option<([[f64; 3]; 3], f64)> {
    let det = det3(m);
    if !(det.abs() > 0.0 && det.is_finite()) {
        return None;
    }
    let mut inv = [[0.0; 3]; 3];
    for (i, row) in inv.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            let (j1, j2) = ((j + 1) % 3, (j + 2) % 3);
            let (i1, i2) = ((i + 1) % 3, (i + 2) % 3);
            // inv[i][j] = cofactor[j][i] / det.
            *v = (m[j1][i1] * m[j2][i2] - m[j1][i2] * m[j2][i1]) / det;
        }
    }
    Some((inv, det))
}

/// The inverse and determinant of a Jacobian that maps a reference element onto a valid one:
/// `None` unless it is admissible by FieldCraft's one validity criterion
/// ([`fieldcraft_model::admissible_jacobian`], the one the meshers' validity gate proves).
pub fn positive_inverse(j: &[[f64; 3]; 3]) -> Option<([[f64; 3]; 3], f64)> {
    if !fieldcraft_model::admissible_jacobian(j) {
        return None;
    }
    inverse3(j)
}

/// The eigenvalues and unit eigenvectors (the columns of the matrix returned) of a symmetric
/// 3 × 3 matrix, by cyclic Jacobi rotations (G. H. Golub, C. F. Van Loan, *Matrix
/// Computations*, 4th ed., Johns Hopkins 2013, §8.5): each rotation zeroes one off-diagonal
/// entry with the stable choice of angle, and the sweeps stop when the off-diagonal part is
/// round-off relative to the diagonal (at most 32 sweeps; three or four suffice). Accurate to a
/// few ulps of the largest eigenvalue; `None` for non-finite input.
pub fn symmetric_eigen(a: &[[f64; 3]; 3]) -> Option<([f64; 3], [[f64; 3]; 3])> {
    if a.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let mut m = [[a[0][0], 0.5 * (a[0][1] + a[1][0]), 0.5 * (a[0][2] + a[2][0])], [0.0; 3], [0.0; 3]];
    m[1] = [m[0][1], a[1][1], 0.5 * (a[1][2] + a[2][1])];
    m[2] = [m[0][2], m[1][2], a[2][2]];
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..32 {
        let off = m[0][1] * m[0][1] + m[0][2] * m[0][2] + m[1][2] * m[1][2];
        let diag = m[0][0] * m[0][0] + m[1][1] * m[1][1] + m[2][2] * m[2][2];
        if off <= 1e-36 * diag || off == 0.0 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            let apq = m[p][q];
            if apq == 0.0 {
                continue;
            }
            // tan of the rotation angle, the smaller root of t² + 2θt − 1 = 0.
            let theta = (m[q][q] - m[p][p]) / (2.0 * apq);
            let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
            let t = sign / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (mkp, mkq) = (m[k][p], m[k][q]);
                m[k][p] = c * mkp - s * mkq;
                m[k][q] = s * mkp + c * mkq;
            }
            for k in 0..3 {
                let (mpk, mqk) = (m[p][k], m[q][k]);
                m[p][k] = c * mpk - s * mqk;
                m[q][k] = s * mpk + c * mqk;
            }
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    Some(([m[0][0], m[1][1], m[2][2]], v))
}

pub fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves_and_rejects_indefinite() {
        let mut a = Matrix::zeros(3);
        a.data = vec![4.0, 2.0, 0.6, 2.0, 5.0, 1.0, 0.6, 1.0, 3.0];
        let x = [1.0, -2.0, 0.5];
        let mut b = a.mul_vec(&x);
        Cholesky::new(&a).unwrap().solve(&mut b);
        assert!(b.iter().zip(x).all(|(p, q)| (p - q).abs() < 1e-14), "{b:?}");
        a.data[8] = -1.0;
        assert!(Cholesky::new(&a).is_err());
        let m = [[2.0, 0.5, 0.0], [0.1, 3.0, 0.2], [0.0, 0.4, 1.0]];
        let (inv, det) = inverse3(&m).unwrap();
        assert!((det - det3(&m)).abs() < 1e-15);
        for i in 0..3 {
            for j in 0..3 {
                let p: f64 = (0..3).map(|k| m[i][k] * inv[k][j]).sum();
                assert!((p - if i == j { 1.0 } else { 0.0 }).abs() < 1e-14);
            }
        }
        assert!(inverse3(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]).is_none());
    }

    #[test]
    fn symmetric_eigen_decomposes() {
        for a in [
            [[4.0, 1.0, -2.0], [1.0, 3.0, 0.5], [-2.0, 0.5, 1.0]],
            [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 5.0]],
            [[1.0, 1e-9, 0.0], [1e-9, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [[1e6, 3.0, 2.0], [3.0, 1e-3, 0.0], [2.0, 0.0, -7.0]],
        ] {
            let (l, v) = symmetric_eigen(&a).unwrap();
            let scale = l.iter().fold(0.0f64, |m, x| m.max(x.abs()));
            for k in 0..3 {
                // A v_k = λ_k v_k and the v_k are orthonormal.
                for i in 0..3 {
                    let av: f64 = (0..3).map(|j| a[i][j] * v[j][k]).sum();
                    assert!((av - l[k] * v[i][k]).abs() < 1e-12 * scale, "{a:?}: {l:?}");
                }
                for m in 0..3 {
                    let d: f64 = (0..3).map(|i| v[i][k] * v[i][m]).sum();
                    assert!((d - if k == m { 1.0 } else { 0.0 }).abs() < 1e-13);
                }
            }
        }
        assert!(symmetric_eigen(&[[f64::NAN, 0.0, 0.0], [0.0; 3], [0.0; 3]]).is_none());
    }
}
