//! Fixed boundary: an already-meshed patch of the body's surface taken as given.
//!
//! A neighbour meshed first (a corner cell mirrored onto its image, a cell next to an existing
//! mesh) leaves quads on the face the two bodies share. The projection mesh of the second body
//! keeps them: the block faces on that face take their divisions from the patch, their nodes are
//! the patch's nodes (held through smoothing), and the two meshes meet node to node.
//!
//! The match is topological. Each block vertex on the patch goes to the nearest patch node; each
//! block edge on it must run along one grid line of the patch from its first vertex's node to its
//! second's (a straight walk: at every node the line goes on through the one neighbour that shares
//! no quad with the edge it came in on); each block face is then filled quad by quad from two of
//! its edges and must close on the other two. Every patch quad belongs to exactly one block face.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::lattice::face_key;
use super::topology::{CORNERS, Topology, corner_at};
use crate::vec3::{P, dist};
use crate::{Error, Result};

/// Most patches one projection takes.
pub const MAX_PATCHES: usize = 64;

/// An already-meshed patch of the body's boundary, taken as given.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FixedPatch {
    /// The B-rep faces of the body the patch covers.
    pub faces: Vec<u32>,
    pub nodes: Vec<P>,
    /// Its quads, by node index (either orientation).
    pub quads: Vec<[u32; 4]>,
}

/// The block faces on the patches, and the patch nodes of their grids.
#[derive(Debug, Clone, Default)]
pub(crate) struct Fixed {
    /// Per topology vertex on a patch, its node's position.
    pub vertex: BTreeMap<u32, P>,
    /// Per block edge on a patch (ascending vertex pair), its divisions.
    pub edge_divisions: BTreeMap<[u32; 2], usize>,
    /// Per block face on a patch, `(block, d, s)` and the positions of its grid: index
    /// `i + (n1 + 1) * j`, `i` along the first other direction of `d`, `j` along the second.
    pub faces: Vec<((usize, usize, usize), Vec<P>)>,
    /// Patch nodes taken.
    pub nodes: usize,
}

/// One patch's quad mesh: the quads round each node and each node's neighbours.
struct Grid<'a> {
    patch: &'a FixedPatch,
    quads_at: Vec<Vec<u32>>,
    nbrs: Vec<BTreeSet<u32>>,
}

impl<'a> Grid<'a> {
    fn new(patch: &'a FixedPatch) -> Result<Grid<'a>> {
        let n = patch.nodes.len();
        let mut quads_at = vec![Vec::new(); n];
        let mut nbrs = vec![BTreeSet::new(); n];
        for (k, q) in patch.quads.iter().enumerate() {
            if q.iter().any(|&v| v as usize >= n) || q.iter().collect::<BTreeSet<_>>().len() != 4 {
                return Err(Error::Invalid(format!("fixed patch quad {k} names a node it does not have, or one twice")));
            }
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                quads_at[a as usize].push(k as u32);
                nbrs[a as usize].insert(b);
                nbrs[b as usize].insert(a);
            }
        }
        Ok(Grid { patch, quads_at, nbrs })
    }

    fn quad(&self, k: u32) -> [u32; 4] {
        self.patch.quads.get(k as usize).copied().unwrap_or([0; 4])
    }

    /// The node after `p` on the grid line coming from `q`: the one neighbour of `p` in no quad
    /// with the edge `q`–`p`.
    fn straight_on(&self, q: u32, p: u32) -> Option<u32> {
        let beside: BTreeSet<u32> =
            self.quads_at.get(p as usize).into_iter().flatten().map(|&k| self.quad(k)).filter(|quad| quad.contains(&q)).flatten().collect();
        let mut on = self.nbrs.get(p as usize)?.iter().filter(|r| **r != q && !beside.contains(r));
        let r = on.next()?;
        on.next().is_none().then_some(*r)
    }

    /// The grid line from `a` to `b`: one of `a`'s neighbours, walked straight on, reaches `b`.
    fn line(&self, a: u32, b: u32) -> Option<Vec<u32>> {
        let cap = self.patch.nodes.len();
        let mut found: Option<Vec<u32>> = None;
        for &first in self.nbrs.get(a as usize)? {
            let mut line = vec![a, first];
            while line.len() <= cap {
                let (Some(&p), Some(&q)) = (line.last(), line.get(line.len().wrapping_sub(2))) else { break };
                if p == b {
                    break;
                }
                match self.straight_on(q, p) {
                    Some(r) => line.push(r),
                    None => break,
                }
            }
            if line.last() == Some(&b) && found.as_ref().is_none_or(|f| line.len() < f.len()) {
                found = Some(line);
            }
        }
        found
    }

    /// The nodes no block face can hold inside: on the patch's boundary, those in other than two
    /// quads (its corners); inside it, those in other than four.
    fn singular(&self) -> Vec<u32> {
        let mut edge_quads: HashMap<[u32; 2], usize> = HashMap::new();
        for q in &self.patch.quads {
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                *edge_quads.entry([a.min(b), a.max(b)]).or_insert(0) += 1;
            }
        }
        let mut on_boundary = vec![false; self.patch.nodes.len()];
        for (e, n) in &edge_quads {
            if *n == 1 {
                for v in e {
                    if let Some(slot) = on_boundary.get_mut(*v as usize) {
                        *slot = true;
                    }
                }
            }
        }
        (0..self.patch.nodes.len() as u32)
            .filter(|&v| {
                let quads = self.quads_at.get(v as usize).map_or(0, Vec::len);
                quads > 0 && quads != if on_boundary.get(v as usize).copied().unwrap_or(false) { 2 } else { 4 }
            })
            .collect()
    }

    /// The nodes of the grid lines from `a`, each walked straight on to its end (at most the
    /// patch's node count per line).
    fn lines_from(&self, a: u32) -> Vec<Vec<u32>> {
        let cap = self.patch.nodes.len();
        let mut out = Vec::new();
        for &first in self.nbrs.get(a as usize).into_iter().flatten() {
            let mut line = vec![a, first];
            while line.len() <= cap {
                let (Some(&p), Some(&q)) = (line.last(), line.get(line.len().wrapping_sub(2))) else { break };
                match self.straight_on(q, p) {
                    Some(r) if r != a => line.push(r),
                    _ => break,
                }
            }
            out.push(line);
        }
        out
    }

    /// The patch node of each block vertex in `vertices` (positions `vpos`, joined by the block
    /// edges `edges`). A node no block face can hold inside is a block vertex: each goes to the
    /// nearest vertex. Then, out from the vertices placed, a vertex joined to a placed one goes
    /// to the node nearest it on the grid lines from the placed one's node. Any left go to their
    /// nearest node.
    fn match_vertices(&self, vertices: &[u32], edges: &BTreeSet<[u32; 2]>, vpos: &[P]) -> BTreeMap<u32, u32> {
        let at = |v: u32| vpos.get(v as usize).copied().unwrap_or_default();
        let node = |n: u32| self.patch.nodes.get(n as usize).copied().unwrap_or_default();
        let mut node_of: BTreeMap<u32, u32> = BTreeMap::new();
        // Singular nodes to vertices, nearest pairs first.
        let mut pairs: Vec<(f64, u32, u32)> =
            self.singular().into_iter().flat_map(|n| vertices.iter().map(move |&v| (n, v))).map(|(n, v)| (dist(node(n), at(v)), v, n)).collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut taken: BTreeSet<u32> = BTreeSet::new();
        for (_, v, n) in pairs {
            if !node_of.contains_key(&v) && !taken.contains(&n) {
                node_of.insert(v, n);
                taken.insert(n);
            }
        }
        loop {
            let mut grew = false;
            for &[a, b] in edges {
                let (from, to) = match (node_of.get(&a), node_of.get(&b)) {
                    (Some(&na), None) => (na, b),
                    (None, Some(&nb)) => (nb, a),
                    _ => continue,
                };
                let best = self
                    .lines_from(from)
                    .into_iter()
                    .flat_map(|l| l.into_iter().skip(1))
                    .filter(|n| !taken.contains(n))
                    .min_by(|x, y| dist(node(*x), at(to)).total_cmp(&dist(node(*y), at(to))));
                if let Some(n) = best {
                    node_of.insert(to, n);
                    taken.insert(n);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        for &v in vertices {
            if let std::collections::btree_map::Entry::Vacant(slot) = node_of.entry(v) {
                let nearest = (0..self.patch.nodes.len() as u32)
                    .filter(|n| !taken.contains(n))
                    .min_by(|x, y| dist(node(*x), at(v)).total_cmp(&dist(node(*y), at(v))));
                if let Some(n) = nearest {
                    slot.insert(n);
                    taken.insert(n);
                }
            }
        }
        node_of
    }

    /// The fourth corner of the quad holding `a`, `b` and `c` (`a` the corner between them), and
    /// the quad.
    fn complete(&self, a: u32, b: u32, c: u32) -> Option<(u32, u32)> {
        let k = *self.quads_at.get(a as usize)?.iter().find(|&&k| {
            let q = self.quad(k);
            q.contains(&b) && q.contains(&c)
        })?;
        let d = self.quad(k).into_iter().find(|v| ![a, b, c].contains(v))?;
        Some((d, k))
    }
}

/// The block-local corner of face `(d, s)` at `(i, j)` (0 or 1) along the face's two directions.
fn face_corner(block: &[u32; 8], d: usize, s: usize, i: usize, j: usize) -> u32 {
    let (e1, e2) = super::other(d);
    let mut bits = [0; 3];
    bits[d] = s;
    bits[e1] = i;
    bits[e2] = j;
    block[corner_at(bits)]
}

/// Match the block faces whose targets lie on the patches: `outer` lists the outer block faces
/// `(block, d, s)`, `target_of` gives a face's target set of B-rep faces, `vpos` the projected
/// vertex positions.
pub(crate) fn match_patches(
    t: &Topology,
    outer: &[(usize, usize, usize)],
    target_of: &dyn Fn(&[u32; 4]) -> Option<Vec<u32>>,
    patches: &[FixedPatch],
    vpos: &[P],
) -> Result<Fixed> {
    if patches.len() > MAX_PATCHES {
        return Err(Error::Invalid(format!("at most {MAX_PATCHES} fixed patches")));
    }
    let mut fixed = Fixed::default();
    for (pi, patch) in patches.iter().enumerate() {
        let grid = Grid::new(patch)?;
        let on: BTreeSet<u32> = patch.faces.iter().copied().collect();
        // The block faces on this patch.
        let mut faces: Vec<(usize, usize, usize)> = Vec::new();
        for &(b, d, s) in outer {
            let Some(block) = t.blocks.get(b) else { continue };
            let Some(set) = target_of(&face_key(block, d, s)) else { continue };
            let inside = set.iter().filter(|f| on.contains(f)).count();
            if inside == 0 {
                continue;
            }
            if inside < set.len() {
                return Err(Error::Invalid(format!(
                    "fixed patch {pi}: face {} of block {b} projects onto B-rep faces {set:?}, only some of them fixed; fix all of its smooth group or none",
                    super::topology::face_number(d, s)
                )));
            }
            faces.push((b, d, s));
        }
        if faces.is_empty() {
            return Err(Error::Invalid(format!("fixed patch {pi}: no block face projects onto its faces {:?}", patch.faces)));
        }
        // Block vertices to patch nodes.
        let mut on_face: Vec<u32> = Vec::new();
        let mut block_edges: BTreeSet<[u32; 2]> = BTreeSet::new();
        for &(b, d, s) in &faces {
            let block = t.blocks.get(b).copied().unwrap_or([0; 8]);
            let cyc = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(i, j)| face_corner(&block, d, s, i, j));
            for k in 0..4 {
                on_face.push(cyc[k]);
                block_edges.insert([cyc[k].min(cyc[(k + 1) % 4]), cyc[k].max(cyc[(k + 1) % 4])]);
            }
        }
        on_face.sort_unstable();
        on_face.dedup();
        let node_of = grid.match_vertices(&on_face, &block_edges, vpos);
        let taken: BTreeSet<u32> = node_of.values().copied().collect();
        if taken.len() != node_of.len() {
            return Err(Error::Failed(format!(
                "fixed patch {pi}: two block vertices fall on one patch node; the blocks there do not match the patch's mesh"
            )));
        }
        // Block edges along grid lines.
        let mut lines: BTreeMap<[u32; 2], Vec<u32>> = BTreeMap::new();
        for &(b, d, s) in &faces {
            let block = t.blocks.get(b).copied().unwrap_or([0; 8]);
            let cyc = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(i, j)| face_corner(&block, d, s, i, j));
            for k in 0..4 {
                let (a, c) = (cyc[k].min(cyc[(k + 1) % 4]), cyc[k].max(cyc[(k + 1) % 4]));
                if lines.contains_key(&[a, c]) {
                    continue;
                }
                let (Some(&na), Some(&nc)) = (node_of.get(&a), node_of.get(&c)) else { continue };
                let line = grid.line(na, nc).ok_or_else(|| {
                    Error::Failed(format!(
                        "fixed patch {pi}: the block edge from vertex {a} to {c} (face {} of block {b}) does not follow a grid line of the patch; the blocks there do not match the patch's mesh",
                        super::topology::face_number(d, s)
                    ))
                })?;
                lines.insert([a, c], line);
            }
        }
        // Each face filled quad by quad from its i and j edges at the 0 corner.
        let mut used: BTreeSet<u32> = BTreeSet::new();
        for &(b, d, s) in &faces {
            let block = t.blocks.get(b).copied().unwrap_or([0; 8]);
            let corner = |i, j| face_corner(&block, d, s, i, j);
            // The edge from corner (i0, j0) to (i1, j1), in that order.
            let edge = |from: u32, to: u32| -> Vec<u32> {
                let line = lines.get(&[from.min(to), from.max(to)]).cloned().unwrap_or_default();
                if from < to { line } else { line.into_iter().rev().collect() }
            };
            let (bottom, left) = (edge(corner(0, 0), corner(1, 0)), edge(corner(0, 0), corner(0, 1)));
            let (top, right) = (edge(corner(0, 1), corner(1, 1)), edge(corner(1, 0), corner(1, 1)));
            let (n1, n2) = (bottom.len().saturating_sub(1), left.len().saturating_sub(1));
            let mismatch = || {
                Error::Failed(format!(
                    "fixed patch {pi}: face {} of block {b} does not close on the patch's mesh ({n1} × {n2} divisions from two of its edges); the blocks there do not match the patch's mesh",
                    super::topology::face_number(d, s)
                ))
            };
            if n1 == 0 || n2 == 0 || top.len() != n1 + 1 || right.len() != n2 + 1 {
                return Err(mismatch());
            }
            let mut g: Vec<u32> = vec![u32::MAX; (n1 + 1) * (n2 + 1)];
            let at = |i: usize, j: usize| i + (n1 + 1) * j;
            for (i, &v) in bottom.iter().enumerate() {
                g[at(i, 0)] = v;
            }
            for (j, &v) in left.iter().enumerate() {
                g[at(0, j)] = v;
            }
            for j in 0..n2 {
                for i in 0..n1 {
                    let (a, x, y) = (g[at(i, j)], g[at(i + 1, j)], g[at(i, j + 1)]);
                    let (dn, k) = grid.complete(a, x, y).ok_or_else(mismatch)?;
                    if !used.insert(k) {
                        return Err(mismatch());
                    }
                    g[at(i + 1, j + 1)] = dn;
                }
            }
            let closes = (0..=n1).all(|i| g[at(i, n2)] == top[i]) && (0..=n2).all(|j| g[at(n1, j)] == right[j]);
            if !closes {
                return Err(mismatch());
            }
            for (pair, line) in [([corner(0, 0), corner(1, 0)], &bottom), ([corner(0, 0), corner(0, 1)], &left)] {
                fixed.edge_divisions.insert([pair[0].min(pair[1]), pair[0].max(pair[1])], line.len() - 1);
            }
            for (pair, line) in [([corner(0, 1), corner(1, 1)], &top), ([corner(1, 0), corner(1, 1)], &right)] {
                fixed.edge_divisions.insert([pair[0].min(pair[1]), pair[0].max(pair[1])], line.len() - 1);
            }
            let positions = g.iter().map(|&v| patch.nodes.get(v as usize).copied().unwrap_or_default()).collect();
            fixed.faces.push(((b, d, s), positions));
        }
        if used.len() != patch.quads.len() {
            return Err(Error::Failed(format!(
                "fixed patch {pi}: the block faces on it cover {} of its {} quads; the patch reaches beyond the faces it names, or the blocks do not match its mesh",
                used.len(),
                patch.quads.len()
            )));
        }
        let mut nodes: BTreeSet<u32> = BTreeSet::new();
        for k in &used {
            nodes.extend(grid.quad(*k));
        }
        fixed.nodes += nodes.len();
        for (v, n) in node_of {
            if let Some(p) = patch.nodes.get(n as usize) {
                fixed.vertex.insert(v, *p);
            }
        }
    }
    Ok(fixed)
}

/// The class divisions the patches set: per class, the count every fixed edge of it has. Edges of
/// one class with different counts on the patches are an error.
pub(crate) fn class_divisions(t: &Topology, classes: &[[usize; 3]], fixed: &Fixed) -> Result<HashMap<usize, usize>> {
    let mut out: HashMap<usize, usize> = HashMap::new();
    for (block, cls) in t.blocks.iter().zip(classes) {
        for (d, &class) in cls.iter().enumerate() {
            for (k, bits) in CORNERS.iter().enumerate() {
                if bits[d] != 0 {
                    continue;
                }
                let mut e = *bits;
                e[d] = 1;
                let (a, b) = (block[k], block[corner_at(e)]);
                let Some(&n) = fixed.edge_divisions.get(&[a.min(b), a.max(b)]) else { continue };
                match out.get(&class) {
                    Some(&m) if m != n => {
                        return Err(Error::Failed(format!(
                            "the fixed patches give parallel block edges {m} and {n} divisions (the edge from vertex {a} to {b}); a structured block mesh cannot meet both"
                        )));
                    }
                    _ => {
                        out.insert(class, n);
                    }
                }
            }
        }
    }
    Ok(out)
}
