//! The adaptive grid: a box of root cubes, each refined as a 27-tree (a leaf splits into
//! 3 × 3 × 3), strongly balanced and closed under the template patterns, then cut into a
//! conforming hexahedral mesh by [`super::templates`].
//!
//! # Balance and closure
//! *Strong balance*: leaves that share a vertex differ by at most one level, so a leaf's finer
//! neighbours are exactly one level finer and a leaf next to a coarser one never has a finer one
//! on that side. A vertex of a leaf at level `L` is *marked* when a finer leaf touches it, that
//! is, when one of the eight level-`L` cells around the vertex is split. *Closure*: a leaf whose
//! marks fit no template (two diagonal corners of a face, three of a face, …) is split too.
//! Balancing and closing repeat until neither splits anything; both only split, so this ends,
//! and every round runs over the leaves in sorted order, so the result is the same on every run.
//!
//! # Coordinates
//! Everything is integer. A cell at level `L` has index `i` (per axis, in units of its own edge)
//! and spans `6 · 3^(D − L)` lattice units, `D` the deepest level allowed; template vertices are
//! in sixths of a leaf, so every vertex is a lattice point and vertices shared by neighbours are
//! found by exact lookup.

use std::collections::HashMap;

use super::templates::{self, Pattern, Sym};
use crate::{Error, Result};

/// A cell: its level and index (in units of its edge at that level).
pub(crate) type Key = (u8, [i64; 3]);

/// The deepest level a tree may have (`3^12` lattice steps per root cell edge, × 6).
pub(crate) const MAX_LEVEL: u8 = 12;

fn pow3(n: u8) -> i64 {
    3i64.pow(u32::from(n))
}

/// What covers a cell-sized region at some level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cover {
    /// A leaf (at that level or coarser).
    Leaf(Key),
    /// The region is split at that level (finer leaves inside).
    Split,
    /// Outside the root box.
    Outside,
}

/// The tree. `nodes` maps every cell that exists to whether it is a leaf.
#[derive(Debug, Clone)]
pub(crate) struct Tree {
    /// Root cubes per axis.
    pub roots: [i64; 3],
    /// The deepest level allowed.
    pub depth: u8,
    nodes: HashMap<Key, bool>,
    leaves: usize,
    max_leaves: usize,
}

/// The sort order of [`Tree::sorted_leaves`]: level, then z, y, x.
fn order(k: &Key) -> (u8, i64, i64, i64) {
    (k.0, k.1[2], k.1[1], k.1[0])
}

/// The 26 neighbour offsets, in a fixed order.
fn offsets() -> impl Iterator<Item = [i64; 3]> {
    (0..27).map(|k| [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1]).filter(|d| *d != [0, 0, 0])
}

impl Tree {
    /// One leaf per root cube.
    pub fn new(roots: [i64; 3], depth: u8, max_leaves: usize) -> Result<Tree> {
        if roots.iter().any(|&n| n < 1) || depth > MAX_LEVEL {
            return Err(Error::Invalid(format!("grid of {roots:?} root cells to depth {depth}")));
        }
        let count = roots.iter().try_fold(1usize, |a, &n| usize::try_from(n).ok().and_then(|n| a.checked_mul(n)));
        let count = count.filter(|&c| c <= max_leaves).ok_or_else(|| Error::Limit(format!("{roots:?} root cells exceed {max_leaves} cells")))?;
        let mut nodes = HashMap::with_capacity(count);
        for z in 0..roots[2] {
            for y in 0..roots[1] {
                for x in 0..roots[0] {
                    nodes.insert((0, [x, y, z]), true);
                }
            }
        }
        Ok(Tree { roots, depth, nodes, leaves: count, max_leaves })
    }

    pub fn leaf_count(&self) -> usize {
        self.leaves
    }

    /// Whether `key` is a leaf.
    pub fn is_leaf(&self, key: &Key) -> bool {
        self.nodes.get(key).copied().unwrap_or(false)
    }

    fn inside(&self, (level, i): Key) -> bool {
        let n = pow3(level);
        (0..3).all(|k| i[k] >= 0 && i[k] < self.roots[k] * n)
    }

    /// What covers the level-`L` cell `key`.
    fn cover(&self, key: Key) -> Cover {
        if !self.inside(key) {
            return Cover::Outside;
        }
        let (level, i) = key;
        for l in (0..=level).rev() {
            let s = pow3(level - l);
            let k = (l, [i[0].div_euclid(s), i[1].div_euclid(s), i[2].div_euclid(s)]);
            match self.nodes.get(&k) {
                Some(true) => return Cover::Leaf(k),
                Some(false) => return Cover::Split,
                None => {}
            }
        }
        Cover::Outside
    }

    /// Split a leaf into its 27 children.
    pub fn split(&mut self, key: Key) -> Result<()> {
        let level = key.0;
        if !self.is_leaf(&key) {
            return Ok(());
        }
        if level >= self.depth {
            return Err(Error::Limit(format!("refinement needs more than {} levels", self.depth)));
        }
        if self.leaves + 26 > self.max_leaves {
            return Err(Error::Limit(format!("the grid needs more than {} cells", self.max_leaves)));
        }
        self.nodes.insert(key, false);
        for child in Self::children(key) {
            self.nodes.insert(child, true);
        }
        self.leaves += 26;
        Ok(())
    }

    /// The leaves in sorted order (level, then z, y, x).
    pub fn sorted_leaves(&self) -> Vec<Key> {
        let mut v: Vec<Key> = self.nodes.iter().filter(|(_, leaf)| **leaf).map(|(k, _)| *k).collect();
        v.sort_by_key(order);
        v
    }

    /// The children of a split cell.
    pub fn children(key: Key) -> impl Iterator<Item = Key> {
        let (level, i) = key;
        (0..27).map(move |k| (level + 1, [3 * i[0] + k % 3, 3 * i[1] + (k / 3) % 3, 3 * i[2] + k / 9]))
    }

    /// Split until leaves sharing a vertex differ by at most one level, checking only the leaves
    /// `work` (in sorted order) and the children of the cells it splits: a split only makes
    /// neighbours finer, so only new leaves can be too fine for a neighbour, and the result is
    /// the unique coarsest balanced refinement whatever the order. Every split cell goes to `log`.
    fn balance_from(&mut self, mut work: Vec<Key>, log: &mut Vec<Key>) -> Result<()> {
        work.reverse();
        while let Some(key) = work.pop() {
            if !self.is_leaf(&key) || key.0 < 2 {
                continue;
            }
            let (level, i) = key;
            for d in offsets() {
                if let Cover::Leaf(k) = self.cover((level, [i[0] + d[0], i[1] + d[1], i[2] + d[2]]))
                    && k.0 + 1 < level
                {
                    self.split(k)?;
                    log.push(k);
                    let mut kids: Vec<Key> = Self::children(k).collect();
                    kids.reverse();
                    work.extend(kids);
                    work.push(key);
                    break;
                }
            }
        }
        Ok(())
    }

    /// The leaves whose marks a split of `key` may change: its children and the leaves beside it
    /// on its level.
    fn touched_by(&self, key: Key, out: &mut Vec<Key>) {
        out.extend(Self::children(key));
        let (level, i) = key;
        out.extend(offsets().map(|d| (level, [i[0] + d[0], i[1] + d[1], i[2] + d[2]])).filter(|k| self.is_leaf(k)));
    }

    /// The marked corners of a leaf (bit `c` for corner `c`: bit 0 x, bit 1 y, bit 2 z).
    pub fn marks(&self, (level, i): Key) -> u8 {
        let mut mask = 0u8;
        for c in 0..8usize {
            let o = [(c & 1) as i64, ((c >> 1) & 1) as i64, ((c >> 2) & 1) as i64];
            let split = (0..8usize).any(|e| {
                let n = [i[0] + o[0] - 1 + (e & 1) as i64, i[1] + o[1] - 1 + ((e >> 1) & 1) as i64, i[2] + o[2] - 1 + ((e >> 2) & 1) as i64];
                self.nodes.get(&(level, n)) == Some(&false)
            });
            if split {
                mask |= 1 << c;
            }
        }
        mask
    }

    /// Balance, then split every leaf whose marks fit no template (and every leaf marked at all
    /// eight corners, whose template is the split), until nothing changes. `max_rounds` bounds the
    /// rounds. Each round re-checks only the leaves the previous splits can have changed, so a
    /// round costs what it splits, not the size of the tree.
    pub fn close(&mut self, max_rounds: usize, skip: &dyn Fn(Key) -> bool) -> Result<()> {
        let mut fresh = self.sorted_leaves();
        let mut touched = fresh.clone();
        for _ in 0..max_rounds {
            let mut log = Vec::new();
            self.balance_from(std::mem::take(&mut fresh), &mut log)?;
            for &k in &log {
                self.touched_by(k, &mut touched);
            }
            touched.retain(|k| self.is_leaf(k));
            touched.sort_by_key(order);
            touched.dedup();
            let bad: Vec<Key> = touched
                .drain(..)
                .filter(|&k| !skip(k))
                .filter(|&k| !matches!(templates::classify(self.marks(k)), Some(Pattern::None | Pattern::Corner | Pattern::Edge | Pattern::Face)))
                .collect();
            if bad.is_empty() {
                return Ok(());
            }
            for &k in &bad {
                self.split(k)?;
            }
            for &k in &bad {
                fresh.extend(Self::children(k));
                self.touched_by(k, &mut touched);
            }
            fresh.sort_by_key(order);
        }
        Err(Error::Limit(format!("grid closure did not settle in {max_rounds} rounds")))
    }

    /// Lattice units per edge of a level-`L` cell.
    pub fn span(&self, level: u8) -> i64 {
        6 * pow3(self.depth - level)
    }

    /// Cut every leaf by its template into a conforming mesh on the integer lattice. Fails if a
    /// leaf's marks fit no template (call [`Tree::close`] first).
    pub fn mesh(&self, skip: &dyn Fn(Key) -> bool) -> Result<LatticeMesh> {
        let syms: Vec<Sym> = templates::symmetries();
        let canon: Vec<(Pattern, Vec<templates::Hex>)> =
            [Pattern::None, Pattern::Corner, Pattern::Edge, Pattern::Face].into_iter().map(|p| (p, templates::template(p))).collect();
        let mut out = LatticeMesh::default();
        let mut index: HashMap<[i64; 3], u32> = HashMap::new();
        for key in self.sorted_leaves() {
            if skip(key) {
                continue;
            }
            let mask = self.marks(key);
            let pattern = templates::classify(mask).filter(|p| *p != Pattern::Full);
            let (pattern, sym) = pattern
                .and_then(|p| templates::orientation(&syms, p, mask).map(|s| (p, s)))
                .ok_or_else(|| Error::Failed(format!("grid cell {key:?} has marks {mask:#010b} that fit no template")))?;
            let hexes = canon.iter().find(|(p, _)| *p == pattern).map(|(_, h)| h.as_slice()).unwrap_or(&[]);
            let (level, i) = key;
            let unit = pow3(self.depth - level);
            for h in hexes {
                let mut g = h.map(|p| {
                    let q = sym.apply(p);
                    [0, 1, 2].map(|k| (6 * i[k] + q[k]) * unit)
                });
                if templates::corner0_orientation(&g) < 0 {
                    g = templates::reversed(g);
                }
                let mut ids = [0u32; 8];
                for (id, p) in ids.iter_mut().zip(g) {
                    *id = match index.get(&p) {
                        Some(&v) => v,
                        None => {
                            let v = u32::try_from(out.points.len()).map_err(|_| Error::Limit("too many grid points".into()))?;
                            index.insert(p, v);
                            out.points.push(p);
                            v
                        }
                    };
                }
                out.hexes.push(ids);
                out.level.push(level);
            }
        }
        Ok(out)
    }
}

/// A conforming hexahedral mesh on the integer lattice.
#[derive(Debug, Clone, Default)]
pub(crate) struct LatticeMesh {
    pub points: Vec<[i64; 3]>,
    pub hexes: Vec<[u32; 8]>,
    /// The tree level of the leaf each hexahedron came from.
    pub level: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::quality::{Validity, element_validity};
    use fieldcraft_model::ElementKind;

    /// A tiny deterministic generator for test trees.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    fn check_conforming(tree: &Tree) {
        let m = tree.mesh(&|_| false).unwrap();
        let mut faces: HashMap<[u32; 4], usize> = HashMap::new();
        let mut volume = 0.0;
        for h in &m.hexes {
            let xyz: Vec<[f64; 3]> = h.iter().map(|&v| m.points[v as usize].map(|c| c as f64)).collect();
            let c = element_validity(ElementKind::Hex8, &xyz).unwrap();
            assert_eq!(c.validity, Validity::Valid);
            volume += c.volume;
            for f in ElementKind::Hex8.faces() {
                let mut q = [h[f[0]], h[f[1]], h[f[2]], h[f[3]]];
                q.sort();
                *faces.entry(q).or_default() += 1;
            }
        }
        let side = tree.span(0) as f64;
        let expected = side.powi(3) * (tree.roots[0] * tree.roots[1] * tree.roots[2]) as f64;
        assert!((volume - expected).abs() < 1e-6 * expected, "volume {volume} vs {expected}");
        let hi = tree.roots.map(|n| n * tree.span(0));
        for (q, n) in faces {
            let on_box = (0..3).any(|k| q.iter().all(|&v| m.points[v as usize][k] == 0) || q.iter().all(|&v| m.points[v as usize][k] == hi[k]));
            assert_eq!(n, if on_box { 1 } else { 2 }, "face {q:?}");
        }
    }

    #[test]
    fn random_trees_close_into_conforming_valid_meshes() {
        let mut rng = Lcg(7);
        for case in 0..12 {
            let roots = [1 + (rng.next() % 2) as i64, 1 + (rng.next() % 2) as i64, 1];
            let mut tree = Tree::new(roots, 3, 1_000_000).unwrap();
            for _ in 0..(1 + rng.next() % 6) {
                let leaves = tree.sorted_leaves();
                let k = leaves[(rng.next() as usize) % leaves.len()];
                if k.0 < 3 {
                    tree.split(k).unwrap();
                }
            }
            tree.close(100, &|_| false).unwrap();
            for k in tree.sorted_leaves() {
                assert!(templates::classify(tree.marks(k)).is_some(), "case {case}");
            }
            check_conforming(&tree);
        }
    }

    #[test]
    fn caps_are_errors() {
        assert!(Tree::new([0, 1, 1], 2, 10).is_err());
        assert!(Tree::new([10, 10, 10], 2, 999).is_err());
        let mut t = Tree::new([1, 1, 1], 1, 100).unwrap();
        t.split((0, [0, 0, 0])).unwrap();
        assert!(t.split((1, [0, 0, 0])).is_err(), "deeper than the depth");
        let mut t = Tree::new([1, 1, 1], 3, 30).unwrap();
        t.split((0, [0, 0, 0])).unwrap();
        assert!(t.split((1, [0, 0, 0])).is_err(), "more cells than allowed");
    }
}
