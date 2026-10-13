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
/// `None` unless det J is positive beyond round-off relative to the product of its row norms.
pub fn positive_inverse(j: &[[f64; 3]; 3]) -> Option<([[f64; 3]; 3], f64)> {
    let (inv, det) = inverse3(j)?;
    let norms: f64 = j.iter().map(|r| (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt()).product();
    (det > 1e-12 * norms).then_some((inv, det))
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
}
