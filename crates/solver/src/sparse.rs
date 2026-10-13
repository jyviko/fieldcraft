//! Sparse symmetric matrices in compressed rows, both triangles stored.
//!
//! Assembly works on the node graph: a node's row holds every node it shares an element with,
//! and each node pair is a dense 3 × 3 block, so the pattern is built once from the element
//! connectivity and the stiffness and mass share it. Sums run in a fixed order (elements by
//! label, then the element's own node order), so assembled values are the same bit for bit on
//! every run and platform.

use crate::{Error, Result};

/// A square sparse matrix in compressed rows with sorted column indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Csr {
    pub n: usize,
    pub ptr: Vec<usize>,
    pub idx: Vec<usize>,
    pub val: Vec<f64>,
}

impl Csr {
    /// The row's column indices and values.
    pub fn row(&self, r: usize) -> (&[usize], &[f64]) {
        let (a, b) = (self.ptr.get(r).copied().unwrap_or(0), self.ptr.get(r + 1).copied().unwrap_or(0));
        (self.idx.get(a..b).unwrap_or_default(), self.val.get(a..b).unwrap_or_default())
    }

    /// y = A x.
    pub fn mul(&self, x: &[f64], y: &mut [f64]) {
        for (r, yr) in y.iter_mut().enumerate().take(self.n) {
            let (cols, vals) = self.row(r);
            *yr = cols.iter().zip(vals).map(|(c, v)| v * x.get(*c).copied().unwrap_or(0.0)).sum();
        }
    }

    pub fn diagonal(&self) -> Vec<f64> {
        (0..self.n)
            .map(|r| {
                let (cols, vals) = self.row(r);
                cols.binary_search(&r).ok().and_then(|i| vals.get(i)).copied().unwrap_or(0.0)
            })
            .collect()
    }

    /// The matrix on a subset of rows and columns, renumbered: `map[i]` is the new index of old
    /// index i, or `None` to drop it. The new rows keep sorted columns.
    pub fn restrict(&self, map: &[Option<usize>], n: usize) -> Csr {
        let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        for (r, m) in map.iter().enumerate() {
            let Some(nr) = *m else { continue };
            let (cols, vals) = self.row(r);
            let Some(out) = rows.get_mut(nr) else { continue };
            for (c, v) in cols.iter().zip(vals) {
                if let Some(Some(nc)) = map.get(*c) {
                    out.push((*nc, *v));
                }
            }
            out.sort_by_key(|e| e.0);
        }
        let mut ptr = Vec::with_capacity(n + 1);
        ptr.push(0);
        let mut idx = Vec::new();
        let mut val = Vec::new();
        for row in rows {
            for (c, v) in row {
                idx.push(c);
                val.push(v);
            }
            ptr.push(idx.len());
        }
        Csr { n, ptr, idx, val }
    }

    /// `self + a·other` on the same pattern.
    pub fn add_scaled(&self, a: f64, other: &Csr) -> Result<Csr> {
        if self.ptr != other.ptr || self.idx != other.idx {
            return Err(Error::Internal("matrices with different patterns".into()));
        }
        let val = self.val.iter().zip(&other.val).map(|(x, y)| x + a * y).collect();
        Ok(Csr { n: self.n, ptr: self.ptr.clone(), idx: self.idx.clone(), val })
    }
}

/// The node-block pattern of a mesh: for each node, the sorted nodes it shares an element with
/// (itself included).
#[derive(Debug, Clone)]
pub struct NodeGraph {
    pub adjacency: Vec<Vec<usize>>,
}

impl NodeGraph {
    pub fn new(nodes: usize, elements: impl Iterator<Item = impl AsRef<[usize]>>) -> NodeGraph {
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); nodes];
        for e in elements {
            let e = e.as_ref();
            for &a in e {
                if let Some(row) = adjacency.get_mut(a) {
                    row.extend_from_slice(e);
                }
            }
        }
        for row in &mut adjacency {
            row.sort_unstable();
            row.dedup();
        }
        NodeGraph { adjacency }
    }

    /// Nonzeros of the dof-level matrix (3 × 3 per node pair).
    pub fn entries(&self) -> usize {
        self.adjacency.iter().map(|r| 9 * r.len()).sum()
    }

    /// A zero matrix over every degree of freedom (3 per node) on this pattern.
    pub fn zero_matrix(&self) -> Csr {
        let n = 3 * self.adjacency.len();
        let mut ptr = Vec::with_capacity(n + 1);
        ptr.push(0);
        let mut idx = Vec::with_capacity(self.entries());
        for row in &self.adjacency {
            for _ in 0..3 {
                for &b in row {
                    idx.extend([3 * b, 3 * b + 1, 3 * b + 2]);
                }
                ptr.push(idx.len());
            }
        }
        let val = vec![0.0; idx.len()];
        Csr { n, ptr, idx, val }
    }
}

/// Where node b's 3 × 3 block starts in each of node a's three rows (the rows of a node share
/// one pattern, built by [`NodeGraph::zero_matrix`]): one search per node pair.
fn block_offsets(k: &Csr, a: usize, b: usize) -> Result<[usize; 3]> {
    let (start, end) = (k.ptr.get(3 * a).copied().unwrap_or(0), k.ptr.get(3 * a + 1).copied().unwrap_or(0));
    let cols = k.idx.get(start..end).ok_or_else(|| Error::Internal("pattern".into()))?;
    let at = cols.binary_search(&(3 * b)).map_err(|_| Error::Internal("entry outside the pattern".into()))?;
    Ok([0, 1, 2].map(|r| k.ptr.get(3 * a + r).copied().unwrap_or(0) + at))
}

/// Add an element matrix (3n × 3n, row-major, degrees of freedom node by node) into a matrix
/// built by [`NodeGraph::zero_matrix`]; `nodes` are the element's global node indices.
pub fn add_block(k: &mut Csr, nodes: &[usize], data: &[f64]) -> Result<()> {
    let width = 3 * nodes.len();
    if data.len() != width * width {
        return Err(Error::Internal("element matrix size".into()));
    }
    for (i, &a) in nodes.iter().enumerate() {
        for (j, &b) in nodes.iter().enumerate() {
            let rows = block_offsets(k, a, b)?;
            for (r, start) in rows.iter().enumerate() {
                for c in 0..3 {
                    if let (Some(x), Some(v)) = (k.val.get_mut(start + c), data.get((3 * i + r) * width + 3 * j + c)) {
                        *x += v;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Add an n × n element matrix to each displacement component's block (consistent mass).
pub fn add_scalar(k: &mut Csr, nodes: &[usize], data: &[f64]) -> Result<()> {
    let n = nodes.len();
    if data.len() != n * n {
        return Err(Error::Internal("element matrix size".into()));
    }
    for (i, &a) in nodes.iter().enumerate() {
        for (j, &b) in nodes.iter().enumerate() {
            let rows = block_offsets(k, a, b)?;
            let v = data.get(i * n + j).copied().unwrap_or(0.0);
            for (c, start) in rows.iter().enumerate() {
                if let Some(x) = k.val.get_mut(start + c) {
                    *x += v;
                }
            }
        }
    }
    Ok(())
}

/// Add a per-node diagonal to each displacement component (lumped mass).
pub fn add_diagonal(k: &mut Csr, nodes: &[usize], diagonal: &[f64]) -> Result<()> {
    for (&a, v) in nodes.iter().zip(diagonal) {
        let rows = block_offsets(k, a, a)?;
        for (c, start) in rows.iter().enumerate() {
            if let Some(x) = k.val.get_mut(start + c) {
                *x += v;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_restrict_multiply() {
        // Two "elements" sharing node 1.
        let g = NodeGraph::new(3, [vec![0usize, 1], vec![1, 2]].into_iter());
        assert_eq!(g.adjacency, vec![vec![0, 1], vec![0, 1, 2], vec![1, 2]]);
        let mut k = g.zero_matrix();
        let mut e = vec![0.0; 36];
        for i in 0..6 {
            e[i * 6 + i] = 2.0;
            e[i * 6 + (i + 3) % 6] = -1.0;
        }
        add_block(&mut k, &[0, 1], &e).unwrap();
        add_block(&mut k, &[1, 2], &e).unwrap();
        assert_eq!(k.diagonal(), vec![2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 2.0, 2.0, 2.0]);
        let x = vec![1.0; 9];
        let mut y = vec![0.0; 9];
        k.mul(&x, &mut y);
        assert_eq!(y, vec![1.0; 3].into_iter().chain([2.0; 3]).chain([1.0; 3]).collect::<Vec<_>>());
        // Keep the x components only, in reverse order.
        let map: Vec<Option<usize>> = (0..9).map(|i| (i % 3 == 0).then_some(2 - i / 3)).collect();
        let r = k.restrict(&map, 3);
        assert_eq!(r.diagonal(), vec![2.0, 4.0, 2.0]);
        assert_eq!(r.row(1).0, &[0, 1, 2]);
        assert!(add_block(&mut k, &[0, 2], &e).is_err(), "nodes 0 and 2 share no element");
        // Scalar (per component) and diagonal additions land on each component.
        let mut m = g.zero_matrix();
        add_scalar(&mut m, &[0, 1], &[2.0, 1.0, 1.0, 2.0]).unwrap();
        add_diagonal(&mut m, &[2], &[5.0]).unwrap();
        assert_eq!(m.diagonal(), vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 5.0, 5.0, 5.0]);
        assert_eq!(m.row(0).1, &[2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let s = k.add_scaled(-1.0, &k).unwrap();
        assert!(s.val.iter().all(|v| *v == 0.0));
    }
}
