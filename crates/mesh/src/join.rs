//! Joining new elements to the mesh already in a model: which new points coincide with existing
//! nodes ([`weld`]), and whether the new elements then meet the mesh face to face
//! ([`check_conforming`]). Mesh generators that add to a model (`mesh.block`) use both; node
//! merging and orphan-mesh import will too.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fieldcraft_model::{ElementKind, Model};

use crate::vec3::{P, dot, sub};
use crate::{Error, Result};

/// A uniform grid of cells of side `size` over items with a bounding box.
struct Grid<T> {
    size: f64,
    cells: HashMap<[i64; 3], Vec<T>>,
}

impl<T: Copy> Grid<T> {
    fn new(size: f64) -> Grid<T> {
        Grid { size, cells: HashMap::new() }
    }

    fn cell(&self, p: P) -> [i64; 3] {
        p.map(|x| (x / self.size).floor() as i64)
    }

    /// Put `item` in every cell its box `[lo, hi]` overlaps.
    fn insert(&mut self, lo: P, hi: P, item: T) {
        let (a, b) = (self.cell(lo), self.cell(hi));
        for i in a[0]..=b[0] {
            for j in a[1]..=b[1] {
                for k in a[2]..=b[2] {
                    self.cells.entry([i, j, k]).or_default().push(item);
                }
            }
        }
    }

    /// The items of the cell holding `p` and of the 26 around it.
    fn near(&self, p: P) -> impl Iterator<Item = T> + '_ {
        let c = self.cell(p);
        (-1..=1_i64).flat_map(move |dx| {
            (-1..=1_i64).flat_map(move |dy| {
                (-1..=1_i64).flat_map(move |dz| {
                    let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                    self.cells.get(&key).into_iter().flatten().copied()
                })
            })
        })
    }
}

fn bounds(points: impl Iterator<Item = P>) -> Option<(P, P)> {
    points.fold(None, |b, p| match b {
        None => Some((p, p)),
        Some((lo, hi)) => Some(([0, 1, 2].map(|k| lo[k].min(p[k])), [0, 1, 2].map(|k| hi[k].max(p[k])))),
    })
}

/// For each point, the model node within `tol` of it: the nearest, the lowest label on a tie.
/// Two points may not weld to the same node. `tol` = 0 welds nothing.
pub fn weld(model: &Model, points: &[P], tol: f64) -> Result<Vec<Option<u32>>> {
    if !(tol.is_finite() && tol >= 0.0) {
        return Err(Error::Invalid(format!("the weld distance must be finite and not negative, not {tol}")));
    }
    let Some((lo, hi)) = bounds(points.iter().copied()).filter(|_| tol > 0.0) else { return Ok(vec![None; points.len()]) };
    let inside = |p: P| (0..3).all(|k| p[k] >= lo[k] - tol && p[k] <= hi[k] + tol);
    let mut grid = Grid::new(tol);
    for (&label, node) in model.nodes().iter().filter(|(_, n)| inside(n.xyz)) {
        grid.insert(node.xyz, node.xyz, (label, node.xyz));
    }
    let mut taken: HashMap<u32, usize> = HashMap::new();
    points
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let best = grid
                .near(p)
                .map(|(label, xyz)| (dot(sub(xyz, p), sub(xyz, p)), label))
                .filter(|(d2, _)| *d2 <= tol * tol)
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, label)| label);
            if let Some(label) = best
                && let Some(other) = taken.insert(label, i)
            {
                return Err(Error::Invalid(format!("points {other} and {i} both lie within {tol} of node {label}: lower the distance or refine")));
            }
            Ok(best)
        })
        .collect()
}

/// A face or edge by its sorted node labels.
fn key(nodes: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut k: Vec<u32> = nodes.collect();
    k.sort_unstable();
    k
}

/// The sides (faces of solids, edges of 2D elements; by sorted corners, with their sorted
/// nodes) and the corner edges of elements.
struct Skin {
    faces: BTreeMap<Vec<u32>, Vec<u32>>,
    edges: BTreeSet<[u32; 2]>,
}

impl Skin {
    /// Every face and corner edge of the elements; with `boundary`, only the faces that occur
    /// once, and the edges of those.
    fn of<'a>(elements: impl Iterator<Item = (ElementKind, &'a [u32])>, boundary: bool) -> Skin {
        let mut count: BTreeMap<Vec<u32>, (usize, Vec<u32>, Vec<u32>)> = BTreeMap::new();
        let mut all_edges = BTreeSet::new();
        for (kind, nodes) in elements {
            let corners = kind.corner_count();
            // The sides that join neighbours: faces of solids, edges of 2D elements.
            let sides = if kind.dimension() == 3 { kind.faces() } else { kind.edges() };
            for local in sides {
                let cs: Vec<u32> = local.iter().filter(|&&i| i < corners).filter_map(|&i| nodes.get(i).copied()).collect();
                let entry = count.entry(key(cs.iter().copied())).or_insert_with(|| (0, key(local.iter().filter_map(|&i| nodes.get(i).copied())), cs));
                entry.0 += 1;
            }
            if !boundary {
                for local in kind.edges() {
                    if let (Some(&a), Some(&b)) = (local.first().and_then(|&i| nodes.get(i)), local.get(1).and_then(|&i| nodes.get(i))) {
                        all_edges.insert([a.min(b), a.max(b)]);
                    }
                }
            }
        }
        let mut faces = BTreeMap::new();
        let mut edges = all_edges;
        for (corners, (n, nodes, cycle)) in count {
            if boundary && n != 1 {
                continue;
            }
            if boundary {
                for (i, &a) in cycle.iter().enumerate() {
                    let b = cycle.get((i + 1) % cycle.len()).copied().unwrap_or(a);
                    edges.insert([a.min(b), a.max(b)]);
                }
            }
            faces.insert(corners, nodes);
        }
        Skin { faces, edges }
    }
}

/// Whether new elements (their nodes already in the model) meet the model's elements face to
/// face where they share the nodes in `joined`: an element face of either side whose corners are
/// all shared must be a face of the other side with the same nodes (no mixed linear and
/// quadratic sides), and no node of the new boundary may lie on an edge of the old elements
/// within `tol`, nor the other way round (no hanging nodes where divisions differ). Edges are
/// tested as straight segments, so along a curved shared edge only the face test applies.
pub fn check_conforming(model: &Model, new: &[(ElementKind, Vec<u32>)], joined: &BTreeSet<u32>, tol: f64) -> Result<()> {
    if joined.is_empty() {
        return Ok(());
    }
    let refuse =
        |what: String| Err(Error::Invalid(format!("{what}: new elements must meet the mesh face to face, with the same divisions and element kind")));
    let new_skin = Skin::of(new.iter().map(|(k, n)| (*k, n.as_slice())), true);
    let touching = model.elements().values().filter(|e| e.nodes.iter().any(|n| joined.contains(n)));
    let old_skin = Skin::of(touching.map(|e| (e.kind, e.nodes.as_slice())), false);
    for (mine, theirs, what) in [(&new_skin, &old_skin, "a new face"), (&old_skin, &new_skin, "an element face")] {
        for (corners, nodes) in &mine.faces {
            if corners.iter().all(|n| joined.contains(n)) && theirs.faces.get(corners) != Some(nodes) {
                return refuse(format!("{what} on nodes {corners:?} lies on shared nodes but the other side has no such face"));
            }
        }
    }
    let xyz = |label: u32| model.node(label).map(|n| n.xyz);
    for (nodes, edges) in [(&new_skin, &old_skin), (&old_skin, &new_skin)] {
        let segments: Vec<([u32; 2], P, P)> = edges.edges.iter().filter_map(|&[a, b]| Some(([a, b], xyz(a)?, xyz(b)?))).collect();
        let Some(longest) = segments.iter().map(|(_, a, b)| dot(sub(*b, *a), sub(*b, *a)).sqrt()).reduce(f64::max).filter(|l| *l > 0.0) else {
            continue;
        };
        let mut grid = Grid::new(longest + 2.0 * tol);
        for (i, (_, a, b)) in segments.iter().enumerate() {
            grid.insert([0, 1, 2].map(|k| a[k].min(b[k]) - tol), [0, 1, 2].map(|k| a[k].max(b[k]) + tol), i);
        }
        let candidates: BTreeSet<u32> = nodes.faces.values().flatten().copied().filter(|n| !joined.contains(n)).collect();
        for n in candidates {
            let Some(p) = xyz(n) else { continue };
            for i in grid.near(p) {
                let Some(&([a, b], xa, xb)) = segments.get(i) else { continue };
                let (ab, ap) = (sub(xb, xa), sub(p, xa));
                let len2 = dot(ab, ab);
                let t = dot(ap, ab) / len2;
                let d = sub(ap, ab.map(|x| t * x));
                let margin = tol / len2.sqrt();
                if t > margin && t < 1.0 - margin && dot(d, d) <= tol * tol {
                    return refuse(format!("node {n} at {p:?} lies on the edge between nodes {a} and {b}"));
                }
            }
        }
    }
    Ok(())
}
