//! Sparse symmetric matrices in compressed rows, both triangles stored.
//!
//! Assembly works on the node graph: a node's row holds every node it shares an element with,
//! and each node pair is a dense block of their degrees of freedom ([`DofLayout`]: a node's are
//! contiguous), so the pattern is built once from the element connectivity and the stiffness and
//! mass share it. Sums run in a fixed order (elements by
//! label, then the element's own node order), so assembled values are the same bit for bit on
//! every run and platform.

use modelcraft_fem::{DofKind, DofLayout, DofSet};

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

    /// Make the matrix exactly symmetric by copying the lower triangle (what the factorization
    /// reads) onto the upper one; the pattern must be structurally symmetric.
    pub fn mirror_lower(&mut self) {
        for r in 0..self.n {
            let (start, end) = (self.ptr.get(r).copied().unwrap_or(0), self.ptr.get(r + 1).copied().unwrap_or(0));
            for at in start..end {
                let Some(&c) = self.idx.get(at) else { continue };
                if c <= r {
                    continue;
                }
                let (cs, ce) = (self.ptr.get(c).copied().unwrap_or(0), self.ptr.get(c + 1).copied().unwrap_or(0));
                let mirror = self.idx.get(cs..ce).and_then(|cols| cols.binary_search(&r).ok()).and_then(|i| self.val.get(cs + i)).copied();
                if let (Some(v), Some(x)) = (mirror, self.val.get_mut(at)) {
                    *x = v;
                }
            }
        }
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

    /// Nonzeros of the dof-level matrix: a dense block per node pair.
    pub fn entries(&self, layout: &DofLayout) -> usize {
        self.adjacency.iter().enumerate().map(|(a, row)| layout.range(a).len() * row.iter().map(|b| layout.range(*b).len()).sum::<usize>()).sum()
    }

    /// A zero matrix over the mesh nodes' degrees of freedom (layout slots `0..nodes`) on this
    /// pattern.
    pub fn zero_matrix(&self, layout: &DofLayout) -> Csr {
        let n = layout.node_dofs();
        let mut ptr = Vec::with_capacity(n + 1);
        ptr.push(0);
        let mut idx = Vec::with_capacity(self.entries(layout));
        for (a, row) in self.adjacency.iter().enumerate() {
            for _ in layout.range(a) {
                for &b in row {
                    idx.extend(layout.range(b));
                }
                ptr.push(idx.len());
            }
        }
        let val = vec![0.0; idx.len()];
        Csr { n, ptr, idx, val }
    }
}

/// Node b's block in node a's rows of a matrix built by [`NodeGraph::zero_matrix`] (the rows of
/// a node share one pattern): one search per node pair.
struct Block<'l> {
    layout: &'l DofLayout,
    a: usize,
    b: usize,
    /// The block's first column within each of a's rows.
    at: usize,
}

impl<'l> Block<'l> {
    fn new(k: &Csr, layout: &'l DofLayout, a: usize, b: usize) -> Result<Block<'l>> {
        let row = layout.range(a).start;
        let (start, end) = (k.ptr.get(row).copied().unwrap_or(0), k.ptr.get(row + 1).copied().unwrap_or(0));
        let cols = k.idx.get(start..end).ok_or_else(|| Error::Internal("pattern".into()))?;
        let at = cols.binary_search(&layout.range(b).start).map_err(|_| Error::Internal("entry outside the pattern".into()))?;
        Ok(Block { layout, a, b, at })
    }

    /// The value index of (a's `row`, b's `col`).
    fn entry(&self, k: &Csr, row: DofKind, col: DofKind) -> Result<usize> {
        let missing = || Error::Internal(format!("an element degree of freedom ({row:?}, {col:?}) its nodes do not have"));
        let r = self.layout.dof(self.a, row).ok_or_else(missing)?;
        let c = self.layout.kinds(self.b).position(col).ok_or_else(missing)?;
        Ok(k.ptr.get(r).copied().unwrap_or(0) + self.at + c)
    }
}

fn add_at(k: &mut Csr, at: usize, v: f64) {
    if let Some(x) = k.val.get_mut(at) {
        *x += v;
    }
}

/// Add an element matrix into a matrix built by [`NodeGraph::zero_matrix`]: row-major over the
/// element's degrees of freedom, node by node and `kinds` in order within a node; `nodes` are
/// the element's layout slots.
pub fn add_block(k: &mut Csr, layout: &DofLayout, nodes: &[usize], kinds: DofSet, data: &[f64]) -> Result<()> {
    let per = kinds.len();
    let width = per * nodes.len();
    if data.len() != width * width {
        return Err(Error::Internal("element matrix size".into()));
    }
    for (i, &a) in nodes.iter().enumerate() {
        for (j, &b) in nodes.iter().enumerate() {
            let block = Block::new(k, layout, a, b)?;
            for (r, row) in kinds.iter().enumerate() {
                for (c, col) in kinds.iter().enumerate() {
                    let at = block.entry(k, row, col)?;
                    add_at(k, at, data.get((per * i + r) * width + per * j + c).copied().unwrap_or(0.0));
                }
            }
        }
    }
    Ok(())
}

/// Add an n × n element matrix to each translation's block (consistent mass).
pub fn add_scalar(k: &mut Csr, layout: &DofLayout, nodes: &[usize], data: &[f64]) -> Result<()> {
    let n = nodes.len();
    if data.len() != n * n {
        return Err(Error::Internal("element matrix size".into()));
    }
    for (i, &a) in nodes.iter().enumerate() {
        for (j, &b) in nodes.iter().enumerate() {
            let block = Block::new(k, layout, a, b)?;
            let v = data.get(i * n + j).copied().unwrap_or(0.0);
            for kind in DofSet::TRANSLATIONS.iter() {
                let at = block.entry(k, kind, kind)?;
                add_at(k, at, v);
            }
        }
    }
    Ok(())
}

/// Add a per-node diagonal to each translation (lumped mass).
pub fn add_diagonal(k: &mut Csr, layout: &DofLayout, nodes: &[usize], diagonal: &[f64]) -> Result<()> {
    for (&a, v) in nodes.iter().zip(diagonal) {
        let block = Block::new(k, layout, a, a)?;
        for kind in DofSet::TRANSLATIONS.iter() {
            let at = block.entry(k, kind, kind)?;
            add_at(k, at, *v);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_mirror_multiply() {
        // Two "elements" sharing node 1.
        let g = NodeGraph::new(3, [vec![0usize, 1], vec![1, 2]].into_iter());
        assert_eq!(g.adjacency, vec![vec![0, 1], vec![0, 1, 2], vec![1, 2]]);
        let layout = DofLayout::uniform(0..3, DofSet::TRANSLATIONS);
        let mut k = g.zero_matrix(&layout);
        let mut e = vec![0.0; 36];
        for i in 0..6 {
            e[i * 6 + i] = 2.0;
            e[i * 6 + (i + 3) % 6] = -1.0;
        }
        add_block(&mut k, &layout, &[0, 1], DofSet::TRANSLATIONS, &e).unwrap();
        add_block(&mut k, &layout, &[1, 2], DofSet::TRANSLATIONS, &e).unwrap();
        assert_eq!(k.diagonal(), vec![2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 2.0, 2.0, 2.0]);
        let x = vec![1.0; 9];
        let mut y = vec![0.0; 9];
        k.mul(&x, &mut y);
        assert_eq!(y, vec![1.0; 3].into_iter().chain([2.0; 3]).chain([1.0; 3]).collect::<Vec<_>>());
        // Mirroring keeps a symmetric matrix as it is.
        let mut mirrored = k.clone();
        mirrored.mirror_lower();
        assert_eq!(mirrored, k);
        assert!(add_block(&mut k, &layout, &[0, 2], DofSet::TRANSLATIONS, &e).is_err(), "nodes 0 and 2 share no element");
        // Scalar (per component) and diagonal additions land on each component.
        let mut m = g.zero_matrix(&layout);
        add_scalar(&mut m, &layout, &[0, 1], &[2.0, 1.0, 1.0, 2.0]).unwrap();
        add_diagonal(&mut m, &layout, &[2], &[5.0]).unwrap();
        assert_eq!(m.diagonal(), vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 5.0, 5.0, 5.0]);
        assert_eq!(m.row(0).1, &[2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let s = k.add_scaled(-1.0, &k).unwrap();
        assert!(s.val.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn mixed_kinds_per_node_assemble_into_their_blocks() {
        // A solid on nodes 0 and 1 (translations) and a beam-like element on 1 and 2 (six each):
        // node 1 carries six, and the solid's block lands on its translations.
        let mut b = DofLayout::builder();
        b.node(10, DofSet::TRANSLATIONS).node(11, DofSet::STRUCTURAL).node(12, DofSet::STRUCTURAL);
        let layout = b.build();
        let g = NodeGraph::new(3, [vec![0usize, 1], vec![1, 2]].into_iter());
        let mut k = g.zero_matrix(&layout);
        assert_eq!((k.n, k.idx.len(), g.entries(&layout)), (15, 3 * 9 + 6 * 15 + 6 * 12, 189));
        let solid: Vec<f64> = (0..36).map(|i| i as f64).collect();
        add_block(&mut k, &layout, &[0, 1], DofSet::TRANSLATIONS, &solid).unwrap();
        let beam = vec![1.0; 144];
        add_block(&mut k, &layout, &[1, 2], DofSet::STRUCTURAL, &beam).unwrap();
        // Row Uy of node 1 (global 4): the solid's row 4 on node 0's translations and node 1's.
        let (cols, vals) = k.row(4);
        let at = |c: usize| vals[cols.binary_search(&c).unwrap()];
        assert_eq!([at(0), at(1), at(2)], [24.0, 25.0, 26.0]);
        assert_eq!([at(3), at(4), at(5)], [27.0 + 1.0, 28.0 + 1.0, 29.0 + 1.0]);
        assert_eq!(at(6), 1.0, "rotation Rx of node 1 from the beam only");
        // A translations element cannot claim rotations its node lacks.
        assert!(add_block(&mut k, &layout, &[0, 1], DofSet::STRUCTURAL, &beam).is_err());
    }
}
