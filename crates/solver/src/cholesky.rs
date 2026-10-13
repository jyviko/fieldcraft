//! Sparse Cholesky factorization A = L Lᵀ of a symmetric positive definite matrix.
//!
//! The up-looking method of T. A. Davis, *Direct Methods for Sparse Linear Systems*, SIAM 2006,
//! §4.1–4.8: the elimination tree (J. W. H. Liu, *The role of elimination trees in sparse
//! factorization*, SIAM J. Matrix Anal. Appl. 11(1):134–172, 1990) gives the pattern of each row
//! of L as a reach in the tree, the symbolic pass counts every column before any value is
//! computed (so the factor's size is checked against a cap before memory is committed), and each
//! row of L is a sparse triangular solve. Operations run in a fixed order, with no threads and no
//! data-dependent reduction order, so the factor is the same bit for bit on every platform.
//!
//! A pivot at or below `PIVOT_TOLERANCE` times the matrix's own diagonal entry means the matrix
//! is singular to working precision (an unconstrained rigid-body motion, a disconnected part, a
//! zero-stiffness direction); the factorization stops and names the column.

use crate::{Error, Result};

/// Relative pivot size below which the matrix counts as singular.
pub const PIVOT_TOLERANCE: f64 = 1e-10;

/// The upper triangle of a symmetric matrix by columns: column k holds A(i, k) for i ≤ k, rows
/// sorted.
#[derive(Debug, Clone)]
pub struct UpperColumns {
    pub n: usize,
    pub ptr: Vec<usize>,
    pub idx: Vec<usize>,
    pub val: Vec<f64>,
}

impl UpperColumns {
    /// From a symmetric matrix stored by full rows (row k's entries with column ≤ k are column k
    /// of the upper triangle).
    pub fn from_symmetric(a: &crate::sparse::Csr) -> UpperColumns {
        let mut ptr = Vec::with_capacity(a.n + 1);
        ptr.push(0);
        let mut idx = Vec::new();
        let mut val = Vec::new();
        for k in 0..a.n {
            let (cols, vals) = a.row(k);
            for (c, v) in cols.iter().zip(vals) {
                if *c <= k {
                    idx.push(*c);
                    val.push(*v);
                }
            }
            ptr.push(idx.len());
        }
        UpperColumns { n: a.n, ptr, idx, val }
    }

    fn column(&self, k: usize) -> (&[usize], &[f64]) {
        let (a, b) = (self.ptr.get(k).copied().unwrap_or(0), self.ptr.get(k + 1).copied().unwrap_or(0));
        (self.idx.get(a..b).unwrap_or_default(), self.val.get(a..b).unwrap_or_default())
    }
}

const NONE: usize = usize::MAX;

/// The elimination tree: parent of each column, `NONE` at roots.
fn etree(a: &UpperColumns) -> Vec<usize> {
    let mut parent = vec![NONE; a.n];
    let mut ancestor = vec![NONE; a.n];
    for k in 0..a.n {
        let (rows, _) = a.column(k);
        for &i0 in rows {
            let mut i = i0;
            // Walk from i up to the root of its current subtree, compressing the path to k.
            while i != NONE && i < k {
                let next = ancestor.get(i).copied().unwrap_or(NONE);
                if let Some(x) = ancestor.get_mut(i) {
                    *x = k;
                }
                if next == NONE
                    && let Some(p) = parent.get_mut(i)
                {
                    *p = k;
                }
                i = next;
            }
        }
    }
    parent
}

/// The pattern of row k of L (excluding the diagonal): the reach of column k's entries in the
/// elimination tree. Fills `stack[top..]` in topological order and returns `top`; `mark` holds
/// `k` for visited columns.
fn ereach(a: &UpperColumns, k: usize, parent: &[usize], stack: &mut [usize], mark: &mut [usize], path: &mut Vec<usize>) -> usize {
    let n = a.n;
    let mut top = n;
    if let Some(m) = mark.get_mut(k) {
        *m = k;
    }
    let (rows, _) = a.column(k);
    for &i0 in rows {
        let mut i = i0;
        if i > k {
            continue;
        }
        path.clear();
        while i != NONE && mark.get(i).copied() != Some(k) {
            path.push(i);
            if let Some(m) = mark.get_mut(i) {
                *m = k;
            }
            i = parent.get(i).copied().unwrap_or(NONE);
        }
        while let Some(v) = path.pop() {
            top -= 1;
            if let Some(s) = stack.get_mut(top) {
                *s = v;
            }
        }
    }
    top
}

/// A Cholesky factor, L by columns with the diagonal first in each column.
#[derive(Debug, Clone)]
pub struct Factor {
    pub n: usize,
    ptr: Vec<usize>,
    idx: Vec<usize>,
    val: Vec<f64>,
}

/// Why a factorization stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum FactorError {
    /// The pivot of this column is not positive (relative to the diagonal).
    NotPositiveDefinite { column: usize },
    /// The factor would hold more entries than allowed.
    TooLarge { entries: usize, limit: usize },
}

impl Factor {
    /// The elimination tree and the entry count of each column of L, without computing values.
    fn symbolic(a: &UpperColumns) -> (Vec<usize>, Vec<usize>) {
        let parent = etree(a);
        let mut counts = vec![1usize; a.n];
        let mut stack = vec![0; a.n];
        let mut mark = vec![NONE; a.n];
        let mut path = Vec::new();
        for k in 0..a.n {
            let top = ereach(a, k, &parent, &mut stack, &mut mark, &mut path);
            for &j in stack.get(top..).unwrap_or_default() {
                if let Some(c) = counts.get_mut(j) {
                    *c += 1;
                }
            }
        }
        (parent, counts)
    }

    pub fn new(a: &UpperColumns, max_entries: usize) -> std::result::Result<Factor, FactorError> {
        let n = a.n;
        let (parent, counts) = Factor::symbolic(a);
        let total: usize = counts.iter().sum();
        if total > max_entries {
            return Err(FactorError::TooLarge { entries: total, limit: max_entries });
        }
        let mut ptr = Vec::with_capacity(n + 1);
        ptr.push(0);
        for c in &counts {
            ptr.push(ptr.last().copied().unwrap_or(0) + c);
        }
        let mut idx = vec![0usize; total];
        let mut val = vec![0.0f64; total];
        // Next free slot in each column.
        let mut next: Vec<usize> = ptr.get(..n).unwrap_or_default().to_vec();
        let mut x = vec![0.0f64; n];
        let mut stack = vec![0usize; n];
        let mut mark = vec![NONE; n];
        let mut path = Vec::new();
        for k in 0..n {
            let top = ereach(a, k, &parent, &mut stack, &mut mark, &mut path);
            let (rows, vals) = a.column(k);
            let mut diag = 0.0;
            for (i, v) in rows.iter().zip(vals) {
                if *i < k {
                    if let Some(xi) = x.get_mut(*i) {
                        *xi = *v;
                    }
                } else if *i == k {
                    diag = *v;
                }
            }
            let mut d = diag;
            for &i in stack.get(top..).unwrap_or_default() {
                let start = ptr.get(i).copied().unwrap_or(0);
                let lii = val.get(start).copied().unwrap_or(1.0);
                let lki = x.get(i).copied().unwrap_or(0.0) / lii;
                if let Some(xi) = x.get_mut(i) {
                    *xi = 0.0;
                }
                let end = next.get(i).copied().unwrap_or(start);
                for p in start + 1..end {
                    let (r, l) = (idx.get(p).copied().unwrap_or(0), val.get(p).copied().unwrap_or(0.0));
                    if let Some(xr) = x.get_mut(r) {
                        *xr -= l * lki;
                    }
                }
                d -= lki * lki;
                if let Some(slot) = next.get_mut(i) {
                    if let (Some(ri), Some(vi)) = (idx.get_mut(*slot), val.get_mut(*slot)) {
                        *ri = k;
                        *vi = lki;
                    }
                    *slot += 1;
                }
            }
            if !(d > PIVOT_TOLERANCE * diag.abs()) || !d.is_finite() {
                return Err(FactorError::NotPositiveDefinite { column: k });
            }
            if let Some(slot) = next.get_mut(k) {
                if let (Some(ri), Some(vi)) = (idx.get_mut(*slot), val.get_mut(*slot)) {
                    *ri = k;
                    *vi = d.sqrt();
                }
                *slot += 1;
            }
        }
        Ok(Factor { n, ptr, idx, val })
    }

    /// Entries in L.
    pub fn entries(&self) -> usize {
        self.val.len()
    }

    /// Solve A x = b in place.
    pub fn solve(&self, b: &mut [f64]) -> Result<()> {
        if b.len() != self.n {
            return Err(Error::Internal("right-hand side length".into()));
        }
        // L y = b, by columns.
        for j in 0..self.n {
            let (start, end) = (self.ptr.get(j).copied().unwrap_or(0), self.ptr.get(j + 1).copied().unwrap_or(0));
            let yj = b.get(j).copied().unwrap_or(0.0) / self.val.get(start).copied().unwrap_or(1.0);
            if let Some(x) = b.get_mut(j) {
                *x = yj;
            }
            for p in start + 1..end {
                let (r, l) = (self.idx.get(p).copied().unwrap_or(0), self.val.get(p).copied().unwrap_or(0.0));
                if let Some(x) = b.get_mut(r) {
                    *x -= l * yj;
                }
            }
        }
        // Lᵀ x = y.
        for j in (0..self.n).rev() {
            let (start, end) = (self.ptr.get(j).copied().unwrap_or(0), self.ptr.get(j + 1).copied().unwrap_or(0));
            let mut s = b.get(j).copied().unwrap_or(0.0);
            for p in start + 1..end {
                let (r, l) = (self.idx.get(p).copied().unwrap_or(0), self.val.get(p).copied().unwrap_or(0.0));
                s -= l * b.get(r).copied().unwrap_or(0.0);
            }
            if let Some(x) = b.get_mut(j) {
                *x = s / self.val.get(start).copied().unwrap_or(1.0);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparse::Csr;

    fn dense_to_csr(a: &[Vec<f64>]) -> Csr {
        let n = a.len();
        let mut ptr = vec![0];
        let mut idx = Vec::new();
        let mut val = Vec::new();
        for row in a {
            for (j, v) in row.iter().enumerate() {
                if *v != 0.0 {
                    idx.push(j);
                    val.push(*v);
                }
            }
            ptr.push(idx.len());
        }
        Csr { n, ptr, idx, val }
    }

    #[test]
    fn factor_and_solve_a_sparse_spd_matrix() {
        // A 2D Laplacian-like matrix on a 7 × 5 grid plus a diagonal shift: sparse with fill.
        let (w, h) = (7, 5);
        let n = w * h;
        let mut a = vec![vec![0.0; n]; n];
        for i in 0..w {
            for j in 0..h {
                let k = i * h + j;
                a[k][k] = 4.5;
                if i + 1 < w {
                    a[k][k + h] = -1.0;
                    a[k + h][k] = -1.0;
                }
                if j + 1 < h {
                    a[k][k + 1] = -1.0;
                    a[k + 1][k] = -1.0;
                }
            }
        }
        let csr = dense_to_csr(&a);
        let f = Factor::new(&UpperColumns::from_symmetric(&csr), usize::MAX).unwrap();
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
        let mut b = vec![0.0; n];
        csr.mul(&x, &mut b);
        f.solve(&mut b).unwrap();
        assert!(b.iter().zip(&x).all(|(p, q)| (p - q).abs() < 1e-13), "{b:?}");
        // The cap is checked before the numeric factorization.
        assert!(matches!(Factor::new(&UpperColumns::from_symmetric(&csr), 10), Err(FactorError::TooLarge { .. })));
        // A singular matrix (a pure Laplacian has the constant vector in its null space).
        let mut sing = a.clone();
        for (k, row) in sing.iter_mut().enumerate() {
            let off: f64 = row.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, v)| -v).sum();
            row[k] = off;
        }
        let e = Factor::new(&UpperColumns::from_symmetric(&dense_to_csr(&sing)), usize::MAX).unwrap_err();
        assert_eq!(e, FactorError::NotPositiveDefinite { column: n - 1 });
    }
}
