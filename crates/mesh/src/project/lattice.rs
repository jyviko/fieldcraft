//! The node lattice of a block topology: each block's structured lattice, with the nodes on
//! shared vertices, edges and faces shared between the blocks.
//!
//! Division counts are per class of parallel block edges (opposite edges of a block have the
//! same count, and so every edge chained to them through other blocks): the classes are the
//! connected components of "parallel in some block". A node on a block's boundary is named by
//! the topology vertices it is blended from and its exact (rational) bilinear weights, which do
//! not depend on which block or orientation sees it, so the blocks around a face, edge or vertex
//! find the same node.

use std::collections::{BTreeMap, HashMap};

use super::topology::{CORNERS, FACE_SIDES, Topology, corner_at};
use crate::{Error, Result};

/// The topological entity a node lies inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entity {
    Vertex(u32),
    /// Ascending vertex pair.
    Edge([u32; 2]),
    /// Ascending corner vertices.
    Face([u32; 4]),
    Inside,
}

/// The ascending corner vertices of block face `(d, s)`.
pub(crate) fn face_key(block: &[u32; 8], d: usize, s: usize) -> [u32; 4] {
    let mut k: Vec<u32> = CORNERS.iter().zip(block).filter(|(c, _)| c[d] == s).map(|(_, v)| *v).collect();
    k.sort_unstable();
    [0, 1, 2, 3].map(|i| k.get(i).copied().unwrap_or(u32::MAX))
}

/// The two corners (block-local) of each block edge along direction `d`.
fn edges_along(d: usize) -> Vec<[usize; 2]> {
    CORNERS
        .iter()
        .enumerate()
        .filter(|(_, c)| c[d] == 0)
        .map(|(k, c)| {
            let mut e = *c;
            e[d] = 1;
            [k, corner_at(e)]
        })
        .collect()
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// The class of parallel edges of each block direction, and the number of classes.
pub(crate) fn edge_classes(t: &Topology) -> (Vec<[usize; 3]>, usize) {
    let mut ids: HashMap<[u32; 2], usize> = HashMap::new();
    let mut id_of = |a: u32, b: u32| {
        let k = [a.min(b), a.max(b)];
        let n = ids.len();
        *ids.entry(k).or_insert(n)
    };
    let mut block_edges: Vec<[[usize; 4]; 3]> = Vec::with_capacity(t.blocks.len());
    for block in &t.blocks {
        let mut per = [[0; 4]; 3];
        for (d, row) in per.iter_mut().enumerate() {
            for (slot, [a, b]) in row.iter_mut().zip(edges_along(d)) {
                *slot = id_of(block[a], block[b]);
            }
        }
        block_edges.push(per);
    }
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    for per in &block_edges {
        for row in per {
            for &e in &row[1..] {
                let (a, b) = (find(&mut parent, row[0]), find(&mut parent, e));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut class: BTreeMap<usize, usize> = BTreeMap::new();
    let out = block_edges
        .iter()
        .map(|per| {
            [0, 1, 2].map(|d| {
                let r = find(&mut parent, per[d][0]);
                let n = class.len();
                *class.entry(r).or_insert(n)
            })
        })
        .collect();
    (out, class.len())
}

/// The structured lattices of the blocks, joined.
#[derive(Debug, Clone)]
pub(crate) struct Lattice {
    pub divisions: Vec<[usize; 3]>,
    /// Per block, the global node of each lattice node ([`crate::quad::lattice_index`]).
    pub block_nodes: Vec<Vec<u32>>,
    pub entity: Vec<Entity>,
    pub hexes: Vec<[u32; 8]>,
    /// The block and cell of each hexahedron.
    pub hex_cell: Vec<(u32, [usize; 3])>,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Build the joined lattices; `divisions[b]` are block `b`'s counts per direction (equal across
/// shared edges, [`edge_classes`]). At most `max_elements` hexahedra.
pub(crate) fn build(t: &Topology, divisions: &[[usize; 3]], max_elements: usize) -> Result<Lattice> {
    if divisions.len() != t.blocks.len() {
        return Err(Error::Invalid("one set of divisions per block".into()));
    }
    let mut total = 0usize;
    for n in divisions {
        if n.iter().any(|&x| x == 0 || x > crate::block::MAX_DIVISIONS) {
            return Err(Error::Invalid(format!("divisions {n:?} out of range")));
        }
        total = total.saturating_add(n[0].saturating_mul(n[1]).saturating_mul(n[2]));
    }
    if total > max_elements {
        return Err(Error::Limit(format!("{total} elements; at most {max_elements} (give a larger size)")));
    }
    let mut keys: HashMap<Vec<(u32, u64, u64)>, u32> = HashMap::new();
    let mut entity: Vec<Entity> = Vec::new();
    let mut block_nodes = Vec::with_capacity(t.blocks.len());
    let mut hexes = Vec::with_capacity(total);
    let mut hex_cell = Vec::with_capacity(total);
    for (b, (block, n)) in t.blocks.iter().zip(divisions).enumerate() {
        let size = (n[0] + 1) * (n[1] + 1) * (n[2] + 1);
        let mut nodes = Vec::with_capacity(size);
        for k in 0..=n[2] {
            for j in 0..=n[1] {
                for i in 0..=n[0] {
                    let at = [i, j, k];
                    let fixed: Vec<(usize, usize)> = (0..3)
                        .filter_map(|d| {
                            if at[d] == 0 {
                                Some((d, 0))
                            } else if at[d] == n[d] {
                                Some((d, 1))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if fixed.is_empty() {
                        nodes.push(entity.len() as u32);
                        entity.push(Entity::Inside);
                        continue;
                    }
                    let free: Vec<usize> = (0..3).filter(|d| !fixed.iter().any(|f| f.0 == *d)).collect();
                    let den: u64 = free.iter().map(|&d| n[d] as u64).product();
                    let mut key: Vec<(u32, u64, u64)> = Vec::new();
                    for (c, bits) in CORNERS.iter().enumerate() {
                        if fixed.iter().any(|&(d, s)| bits[d] != s) {
                            continue;
                        }
                        let num: u64 = free.iter().map(|&d| if bits[d] == 1 { at[d] as u64 } else { (n[d] - at[d]) as u64 }).product();
                        if num == 0 {
                            continue;
                        }
                        let g = gcd(num, den);
                        key.push((block[c], num / g, den / g));
                    }
                    key.sort_unstable();
                    let next = entity.len() as u32;
                    let id = match keys.get(&key) {
                        Some(&id) => id,
                        None => {
                            let mut vs: Vec<u32> = key.iter().map(|x| x.0).collect();
                            vs.sort_unstable();
                            let e = match vs.len() {
                                1 => Entity::Vertex(vs[0]),
                                2 => Entity::Edge([vs[0], vs[1]]),
                                _ => Entity::Face([0, 1, 2, 3].map(|q| vs.get(q).copied().unwrap_or(u32::MAX))),
                            };
                            entity.push(e);
                            keys.insert(key, next);
                            next
                        }
                    };
                    nodes.push(id);
                }
            }
        }
        let idx = |c: [usize; 3]| crate::quad::lattice_index(*n, c);
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let h = CORNERS.map(|c| nodes.get(idx([i + c[0], j + c[1], k + c[2]])).copied().unwrap_or(0));
                    hexes.push(h);
                    hex_cell.push((b as u32, [i, j, k]));
                }
            }
        }
        block_nodes.push(nodes);
    }
    Ok(Lattice { divisions: divisions.to_vec(), block_nodes, entity, hexes, hex_cell })
}

/// How many blocks use each block face.
pub(crate) fn face_uses(t: &Topology) -> Result<HashMap<[u32; 4], usize>> {
    let mut uses: HashMap<[u32; 4], usize> = HashMap::new();
    for block in &t.blocks {
        for (d, s) in FACE_SIDES {
            *uses.entry(face_key(block, d, s)).or_insert(0) += 1;
        }
    }
    if uses.values().any(|&u| u > 2) {
        return Err(Error::Invalid("a block face is shared by more than two blocks".into()));
    }
    Ok(uses)
}
